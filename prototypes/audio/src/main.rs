use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

const TARGET_SAMPLE_RATE: u32 = 16_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TinyVox — Audio PoC");
    println!("====================\n");

    let host = cpal::default_host();

    let device = host
        .default_input_device()
        .ok_or("No default input device found")?;

    println!("Input device: {}", device.description()?);

    let supported_config = device.default_input_config()?;

    println!(
        "Native config: {} Hz, {} channels, {:?}",
        supported_config.sample_rate(),
        supported_config.channels(),
        supported_config.sample_format()
    );

    let sample_rate = supported_config.sample_rate();
    let channels = supported_config.channels();

    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let samples_for_callback = Arc::clone(&samples);

    let error_callback = |error| {
        eprintln!("Audio stream error: {error}");
    };

    let config: StreamConfig = supported_config.clone().into();

    let stream = match supported_config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| {
                if let Ok(mut buffer) = samples_for_callback.lock() {
                    buffer.extend_from_slice(data);
                }
            },
            error_callback,
            None,
        )?,

        SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                if let Ok(mut buffer) = samples_for_callback.lock() {
                    buffer.extend(data.iter().map(|&sample| {
                        sample as f32 / i16::MAX as f32
                    }));
                }
            },
            error_callback,
            None,
        )?,

        SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                if let Ok(mut buffer) = samples_for_callback.lock() {
                    buffer.extend(data.iter().map(|&sample| {
                        (sample as f32 / u16::MAX as f32) * 2.0 - 1.0
                    }));
                }
            },
            error_callback,
            None,
        )?,

        format => {
            return Err(format!("Unsupported sample format: {format:?}").into());
        }
    };

    stream.play()?;

    println!("\nRecording...");
    println!("Speak for a few seconds.");
    print!("Press ENTER to stop: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    drop(stream);

    let captured = samples
        .lock()
        .map_err(|_| "Audio sample buffer was poisoned")?
        .clone();

    println!(
        "\nCaptured {} samples ({:.2} seconds)",
        captured.len(),
        captured.len() as f32 / sample_rate as f32 / channels as f32
    );

    let mono = downmix_to_mono(&captured, channels);

    let resampled = resample_linear(
        &mono,
        sample_rate,
        TARGET_SAMPLE_RATE,
    );

    println!(
        "Resampled: {} samples ({:.2} seconds)",
        resampled.len(),
        resampled.len() as f32 / TARGET_SAMPLE_RATE as f32
    );

    write_wav(&resampled, TARGET_SAMPLE_RATE, "tinyvox-test.wav")?;

    println!("\nWrote: tinyvox-test.wav");

    Ok(())
}

fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }

    samples
        .chunks_exact(channels as usize)
        .map(|frame| {
            frame.iter().copied().sum::<f32>() / channels as f32
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

        let fraction = source_position - left_index as f64;

        let left = samples[left_index];
        let right = samples[right_index];

        let interpolated =
            left + (right - left) * fraction as f32;

        output.push(interpolated);
    }

    output
}

fn write_wav(
    samples: &[f32],
    sample_rate: u32,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)?;

    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);

        let pcm = (clamped * i16::MAX as f32) as i16;

        writer.write_sample(pcm)?;
    }

    writer.finalize()?;

    Ok(())
}