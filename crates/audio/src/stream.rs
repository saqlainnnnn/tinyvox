use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug)]
pub enum AudioStreamError {
    NoInputDevice,
    Cpal(cpal::Error),
    UnsupportedSampleFormat(SampleFormat),
    BufferPoisoned,
    NotStreaming,
}

impl std::fmt::Display for AudioStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputDevice => {
                write!(f, "no input device available")
            }

            Self::Cpal(error) => {
                write!(f, "CPAL audio error: {error}")
            }

            Self::UnsupportedSampleFormat(format) => {
                write!(f, "unsupported input sample format: {format:?}")
            }

            Self::BufferPoisoned => {
                write!(f, "audio buffer lock was poisoned")
            }

            Self::NotStreaming => {
                write!(f, "audio streamer is not active")
            }
        }
    }
}

impl std::error::Error for AudioStreamError {}

pub struct CpalAudioStreamer {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: Option<Stream>,
    sent_samples: usize,
}

impl CpalAudioStreamer {
    pub fn new() -> Result<Self, AudioStreamError> {
        let host = cpal::default_host();

        let device = host
            .default_input_device()
            .ok_or(AudioStreamError::NoInputDevice)?;

        let supported_config = device
            .default_input_config()
            .map_err(AudioStreamError::Cpal)?;

        let config: StreamConfig = supported_config.clone().into();

        Ok(Self {
            device,
            config,
            sample_format: supported_config.sample_format(),
            buffer: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            sent_samples: 0,
        })
    }

    pub fn start(&mut self) -> Result<(), AudioStreamError> {
        if self.stream.is_some() {
            return Ok(());
        }

        self.buffer
            .lock()
            .map_err(|_| AudioStreamError::BufferPoisoned)?
            .clear();

        self.sent_samples = 0;

        let buffer = Arc::clone(&self.buffer);

        let error_callback = |error| {
            eprintln!("TinyVox streaming audio error: {error}");
        };

        let stream = match self.sample_format {
            SampleFormat::F32 => {
                let buffer = Arc::clone(&buffer);

                self.device.build_input_stream(
                    self.config,
                    move |data: &[f32], _| {
                        if let Ok(mut samples) = buffer.lock() {
                            samples.extend_from_slice(data);
                        }
                    },
                    error_callback,
                    None,
                )
            }

            SampleFormat::I16 => {
                let buffer = Arc::clone(&buffer);

                self.device.build_input_stream(
                    self.config,
                    move |data: &[i16], _| {
                        if let Ok(mut samples) = buffer.lock() {
                            samples
                                .extend(data.iter().map(|&sample| sample as f32 / i16::MAX as f32));
                        }
                    },
                    error_callback,
                    None,
                )
            }

            SampleFormat::U16 => {
                let buffer = Arc::clone(&buffer);

                self.device.build_input_stream(
                    self.config,
                    move |data: &[u16], _| {
                        if let Ok(mut samples) = buffer.lock() {
                            samples.extend(
                                data.iter()
                                    .map(|&sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                            );
                        }
                    },
                    error_callback,
                    None,
                )
            }

            format => {
                return Err(AudioStreamError::UnsupportedSampleFormat(format));
            }
        }
        .map_err(AudioStreamError::Cpal)?;

        stream.play().map_err(AudioStreamError::Cpal)?;

        self.stream = Some(stream);

        Ok(())
    }

    pub fn read_chunk(&mut self) -> Result<Vec<u8>, AudioStreamError> {
        if self.stream.is_none() {
            return Err(AudioStreamError::NotStreaming);
        }

        let samples = self
            .buffer
            .lock()
            .map_err(|_| AudioStreamError::BufferPoisoned)?
            .clone();

        let mono = downmix_to_mono(&samples, self.config.channels as usize);

        let resampled = resample_linear(&mono, self.config.sample_rate, TARGET_SAMPLE_RATE);

        if self.sent_samples >= resampled.len() {
            return Ok(Vec::new());
        }

        let new_samples = &resampled[self.sent_samples..];

        self.sent_samples = resampled.len();

        Ok(f32_to_pcm16(new_samples))
    }

    pub fn stop(&mut self) {
        self.stream.take();
        self.sent_samples = 0;
    }
}

fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }

    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
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

fn f32_to_pcm16(samples: &[f32]) -> Vec<u8> {
    let mut output = Vec::with_capacity(samples.len() * 2);

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);

        let value = (clamped * i16::MAX as f32) as i16;

        output.extend_from_slice(&value.to_le_bytes());
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_f32_to_pcm16() {
        let pcm = f32_to_pcm16(&[-1.0, 0.0, 1.0]);

        assert_eq!(pcm.len(), 6);

        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1],]), i16::MIN + 1);

        assert_eq!(i16::from_le_bytes([pcm[2], pcm[3],]), 0);

        assert_eq!(i16::from_le_bytes([pcm[4], pcm[5],]), i16::MAX);
    }

    #[test]
    fn mono_passthrough() {
        let samples = vec![0.1, 0.2, 0.3];

        assert_eq!(downmix_to_mono(&samples, 1,), samples);
    }

    #[test]
    fn stereo_downmixes() {
        let samples = vec![1.0, -1.0, 0.5, 0.5];

        assert_eq!(downmix_to_mono(&samples, 2,), vec![0.0, 0.5,]);
    }
}
