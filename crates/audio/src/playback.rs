use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

const GEMINI_SAMPLE_RATE: u32 = 24_000;

#[derive(Debug)]
pub enum AudioPlaybackError {
    NoOutputDevice,
    Cpal(cpal::Error),
    UnsupportedSampleFormat(SampleFormat),
    BufferPoisoned,
    NotPlaying,
}

impl std::fmt::Display for AudioPlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutputDevice => {
                write!(f, "no output device available")
            }

            Self::Cpal(error) => {
                write!(f, "CPAL playback error: {error}")
            }

            Self::UnsupportedSampleFormat(format) => {
                write!(f, "unsupported output sample format: {format:?}")
            }

            Self::BufferPoisoned => {
                write!(f, "playback buffer lock was poisoned")
            }

            Self::NotPlaying => {
                write!(f, "playback stream is not active")
            }
        }
    }
}

impl std::error::Error for AudioPlaybackError {}

pub struct CpalAudioPlayback {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,

    queue: Arc<Mutex<Vec<f32>>>,

    position: Arc<Mutex<usize>>,

    stream: Option<Stream>,
}

impl CpalAudioPlayback {
    pub fn new() -> Result<Self, AudioPlaybackError> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or(AudioPlaybackError::NoOutputDevice)?;

        let supported_config = device
            .default_output_config()
            .map_err(AudioPlaybackError::Cpal)?;

        let config: StreamConfig = supported_config.clone().into();

        Ok(Self {
            device,
            config,
            sample_format: supported_config.sample_format(),

            queue: Arc::new(Mutex::new(Vec::new())),

            position: Arc::new(Mutex::new(0)),

            stream: None,
        })
    }

    pub fn start(&mut self) -> Result<(), AudioPlaybackError> {
        if self.stream.is_some() {
            return Ok(());
        }

        {
            let mut queue = self
                .queue
                .lock()
                .map_err(|_| AudioPlaybackError::BufferPoisoned)?;

            queue.clear();
        }

        {
            let mut position = self
                .position
                .lock()
                .map_err(|_| AudioPlaybackError::BufferPoisoned)?;

            *position = 0;
        }

        let queue = Arc::clone(&self.queue);

        let position = Arc::clone(&self.position);

        let output_sample_rate = self.config.sample_rate;

        let channels = self.config.channels as usize;

        let error_callback = |error| {
            eprintln!("TinyVox playback stream error: {error}");
        };

        let stream = match self.sample_format {
            SampleFormat::F32 => {
                let queue = Arc::clone(&queue);

                let position = Arc::clone(&position);

                self.device.build_output_stream(
                    self.config.clone(),
                    move |data: &mut [f32], _| {
                        fill_output_f32(data, &queue, &position, channels, output_sample_rate);
                    },
                    error_callback,
                    None,
                )
            }

            SampleFormat::I16 => {
                let queue = Arc::clone(&queue);

                let position = Arc::clone(&position);

                self.device.build_output_stream(
                    self.config.clone(),
                    move |data: &mut [i16], _| {
                        fill_output_i16(data, &queue, &position, channels, output_sample_rate);
                    },
                    error_callback,
                    None,
                )
            }

            SampleFormat::U16 => {
                let queue = Arc::clone(&queue);

                let position = Arc::clone(&position);

                self.device.build_output_stream(
                    self.config.clone(),
                    move |data: &mut [u16], _| {
                        fill_output_u16(data, &queue, &position, channels, output_sample_rate);
                    },
                    error_callback,
                    None,
                )
            }

            format => {
                return Err(AudioPlaybackError::UnsupportedSampleFormat(format));
            }
        }
        .map_err(AudioPlaybackError::Cpal)?;

        stream.play().map_err(AudioPlaybackError::Cpal)?;

        self.stream = Some(stream);

        Ok(())
    }

    pub fn push_pcm16(&mut self, pcm: &[u8]) -> Result<(), AudioPlaybackError> {
        if self.stream.is_none() {
            return Err(AudioPlaybackError::NotPlaying);
        }

        // PCM16 consists of 2 bytes per sample.
        // Ignore a dangling final byte rather
        // than producing a malformed sample.
        let even_len = pcm.len() - pcm.len() % 2;

        if even_len == 0 {
            return Ok(());
        }

        let samples = pcm16_to_f32(&pcm[..even_len]);

        let resampled = resample_linear(&samples, GEMINI_SAMPLE_RATE, self.config.sample_rate);

        let mut queue = self
            .queue
            .lock()
            .map_err(|_| AudioPlaybackError::BufferPoisoned)?;

        queue.extend(resampled.iter().copied());

        Ok(())
    }

    pub fn queued_samples(&self) -> Result<usize, AudioPlaybackError> {
        let queue = self
            .queue
            .lock()
            .map_err(|_| AudioPlaybackError::BufferPoisoned)?;

        let position = *self
            .position
            .lock()
            .map_err(|_| AudioPlaybackError::BufferPoisoned)?;

        Ok(queue.len().saturating_sub(position))
    }

    pub fn stop(&mut self) {
        self.stream.take();

        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
        }

        if let Ok(mut position) = self.position.lock() {
            *position = 0;
        }
    }
}

fn pcm16_to_f32(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(2)
        .map(|bytes| {
            let sample = i16::from_le_bytes([bytes[0], bytes[1]]);

            sample as f32 / i16::MAX as f32
        })
        .collect()
}

fn resample_linear(samples: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if samples.is_empty() || input_rate == output_rate {
        return samples.to_vec();
    }

    let ratio = input_rate as f64 / output_rate as f64;

    let output_len = (samples.len() as f64 / ratio).ceil() as usize;

    let mut output = Vec::with_capacity(output_len);

    for output_index in 0..output_len {
        let source_position = output_index as f64 * ratio;

        let left_index = source_position.floor() as usize;

        if left_index >= samples.len() {
            break;
        }

        let right_index = (left_index + 1).min(samples.len() - 1);

        let fraction = (source_position - left_index as f64) as f32;

        let left = samples[left_index];

        let right = samples[right_index];

        output.push(left + (right - left) * fraction);
    }

    output
}

fn read_sample(queue: &[f32], position: usize) -> f32 {
    queue.get(position).copied().unwrap_or(0.0)
}

fn advance_position(position: &mut usize, queue_len: usize) {
    if *position < queue_len {
        *position += 1;
    }
}

fn fill_output_f32(
    output: &mut [f32],
    queue: &Arc<Mutex<Vec<f32>>>,
    position: &Arc<Mutex<usize>>,
    channels: usize,
    _sample_rate: u32,
) {
    let mut position = match position.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(0.0);
            return;
        }
    };

    let queue = match queue.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(0.0);
            return;
        }
    };

    for frame in output.chunks_mut(channels) {
        let sample = read_sample(&queue, *position);

        for value in frame.iter_mut() {
            *value = sample;
        }

        advance_position(&mut position, queue.len());
    }
}

fn fill_output_i16(
    output: &mut [i16],
    queue: &Arc<Mutex<Vec<f32>>>,
    position: &Arc<Mutex<usize>>,
    channels: usize,
    _sample_rate: u32,
) {
    let mut position = match position.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(0);
            return;
        }
    };

    let queue = match queue.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(0);
            return;
        }
    };

    for frame in output.chunks_mut(channels) {
        let sample = read_sample(&queue, *position).clamp(-1.0, 1.0);

        let value = (sample * i16::MAX as f32) as i16;

        for output_sample in frame.iter_mut() {
            *output_sample = value;
        }

        advance_position(&mut position, queue.len());
    }
}

fn fill_output_u16(
    output: &mut [u16],
    queue: &Arc<Mutex<Vec<f32>>>,
    position: &Arc<Mutex<usize>>,
    channels: usize,
    _sample_rate: u32,
) {
    let mut position = match position.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(u16::MIN);
            return;
        }
    };

    let queue = match queue.lock() {
        Ok(value) => value,

        Err(_) => {
            output.fill(u16::MIN);
            return;
        }
    };

    for frame in output.chunks_mut(channels) {
        let sample = read_sample(&queue, *position).clamp(-1.0, 1.0);

        let value = ((sample + 1.0) * 0.5 * u16::MAX as f32) as u16;

        for output_sample in frame.iter_mut() {
            *output_sample = value;
        }

        advance_position(&mut position, queue.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_zero_is_zero() {
        let pcm = 0i16.to_le_bytes();

        let samples = pcm16_to_f32(&pcm);

        assert_eq!(samples, vec![0.0]);
    }

    #[test]
    fn pcm16_positive_is_positive() {
        let pcm = 16_384i16.to_le_bytes();

        let samples = pcm16_to_f32(&pcm);

        assert!(samples[0] > 0.0);
    }

    #[test]
    fn pcm16_negative_is_negative() {
        let pcm = (-16_384i16).to_le_bytes();

        let samples = pcm16_to_f32(&pcm);

        assert!(samples[0] < 0.0);
    }

    #[test]
    fn resampling_changes_length() {
        let input = vec![0.0; 24_000];

        let output = resample_linear(&input, 24_000, 48_000);

        assert!(output.len() >= 47_999);
    }

    #[test]
    fn odd_pcm_byte_is_ignored() {
        let pcm = vec![0, 0, 123];

        let samples = pcm16_to_f32(&pcm[..2]);

        assert_eq!(samples.len(), 1);
    }
}
