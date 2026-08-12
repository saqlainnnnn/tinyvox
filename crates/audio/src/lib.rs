use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

use tinyvox_engine::ports::{AudioBuffer, AudioRecorder};

pub mod stream;

pub use stream::{
    AudioStreamError,
    CpalAudioStreamer,
};

pub mod playback;

pub use playback::{
    AudioPlaybackError,
    CpalAudioPlayback,
};

const TARGET_SAMPLE_RATE: u32 = 16_000;

pub struct CpalAudioRecorder {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: Option<Stream>,
}

impl CpalAudioRecorder {
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();

        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;

        let supported_config = device
            .default_input_config()
            .map_err(AudioError::Cpal)?;

        let config: StreamConfig = supported_config.clone().into();

        Ok(Self {
            device,
            config,
            sample_format: supported_config.sample_format(),
            buffer: Arc::new(Mutex::new(Vec::new())),
            stream: None,
        })
    }

    fn input_sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    fn channels(&self) -> usize {
        self.config.channels as usize
    }
}

#[derive(Debug)]
pub enum AudioError {
    NoInputDevice,
    Cpal(cpal::Error),
    UnsupportedSampleFormat(SampleFormat),
    BufferPoisoned,
    NotRecording,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputDevice => {
                write!(f, "no input device available")
            }

            Self::Cpal(error) => {
                write!(f, "CPAL audio error: {error}")
            }

            Self::UnsupportedSampleFormat(format) => {
                write!(
                    f,
                    "unsupported input sample format: {format:?}"
                )
            }

            Self::BufferPoisoned => {
                write!(f, "audio buffer lock was poisoned")
            }

            Self::NotRecording => {
                write!(f, "audio recorder is not recording")
            }
        }
    }
}

impl std::error::Error for AudioError {}

impl AudioRecorder for CpalAudioRecorder {
    type Error = AudioError;

    fn start(&mut self) -> Result<(), Self::Error> {
        if self.stream.is_some() {
            return Ok(());
        }

        self.buffer
            .lock()
            .map_err(|_| AudioError::BufferPoisoned)?
            .clear();

        let buffer = Arc::clone(&self.buffer);

        let error_callback = |error| {
            eprintln!("TinyVox audio stream error: {error}");
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
                            samples.extend(data.iter().map(|&sample| {
                                sample as f32 / i16::MAX as f32
                            }));
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
                            samples.extend(data.iter().map(|&sample| {
                                (sample as f32 / u16::MAX as f32) * 2.0 - 1.0
                            }));
                        }
                    },
                    error_callback,
                    None,
                )
            }

            format => {
                return Err(AudioError::UnsupportedSampleFormat(format));
            }
        }
        .map_err(AudioError::Cpal)?;

        stream.play().map_err(AudioError::Cpal)?;

        self.stream = Some(stream);

        Ok(())
    }

    fn stop(&mut self) -> Result<AudioBuffer, Self::Error> {
        let stream = self
            .stream
            .take()
            .ok_or(AudioError::NotRecording)?;

        drop(stream);

        let samples = self
            .buffer
            .lock()
            .map_err(|_| AudioError::BufferPoisoned)?
            .clone();

        let mono = downmix_to_mono(
            &samples,
            self.channels(),
        );

        let resampled = resample_linear(
            &mono,
            self.input_sample_rate(),
            TARGET_SAMPLE_RATE,
        );

        Ok(AudioBuffer {
            samples: resampled,
            sample_rate: TARGET_SAMPLE_RATE,
        })
    }
}

fn downmix_to_mono(
    samples: &[f32],
    channels: usize,
) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }

    samples
        .chunks_exact(channels)
        .map(|frame| {
            frame.iter().copied().sum::<f32>()
                / channels as f32
        })
        .collect()
}

fn resample_linear(
    samples: &[f32],
    input_rate: u32,
    output_rate: u32,
) -> Vec<f32> {
    if samples.is_empty() || input_rate == output_rate {
        return samples.to_vec();
    }

    let ratio =
        input_rate as f64 / output_rate as f64;

    let output_len =
        (samples.len() as f64 / ratio).ceil() as usize;

    let mut output =
        Vec::with_capacity(output_len);

    for output_index in 0..output_len {
        let source_position =
            output_index as f64 * ratio;

        let left_index =
            source_position.floor() as usize;

        if left_index >= samples.len() {
            break;
        }

        let right_index =
            (left_index + 1).min(samples.len() - 1);

        let fraction =
            (source_position - left_index as f64) as f32;

        let left = samples[left_index];
        let right = samples[right_index];

        output.push(
            left + (right - left) * fraction
        );
    }

    output
}