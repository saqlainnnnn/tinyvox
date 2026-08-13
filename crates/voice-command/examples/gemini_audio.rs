use std::{env, fs};

use dotenvy::dotenv;

use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymphoniaError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

use voice_command::{AudioChunk, GeminiLiveProvider, VoiceEvent, VoiceProvider, VoiceSession};

fn decode_mp3(path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();

    if path.to_ascii_lowercase().ends_with(".mp3") {
        hint.with_extension("mp3");
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    let track = format.default_track().ok_or("no audio track found")?;

    let track_id = track.id;

    let codec_params = track.codec_params.clone();

    let mut decoder =
        symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;

    let mut pcm = Vec::<i16>::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,

            Err(SymphoniaError::ResetRequired) => {
                break;
            }

            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }

            Err(error) => return Err(error.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet)?;

        let spec = *decoded.spec();

        let capacity = decoded.capacity() as u64;

        let mut sample_buffer = SampleBuffer::<i16>::new(capacity, spec);

        sample_buffer.copy_interleaved_ref(decoded);

        pcm.extend_from_slice(sample_buffer.samples());
    }

    let channels = codec_params
        .channels
        .map(|channels| channels.count())
        .unwrap_or(1);

    let sample_rate = codec_params.sample_rate.unwrap_or(16_000);

    let mono = if channels > 1 {
        pcm.chunks(channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|&sample| sample as i32).sum();

                (sum / frame.len() as i32) as i16
            })
            .collect::<Vec<i16>>()
    } else {
        pcm
    };

    let resampled = if sample_rate == 16_000 {
        mono
    } else {
        resample_to_16khz(&mono, sample_rate)
    };

    let mut bytes = Vec::with_capacity(resampled.len() * 2);

    for sample in resampled {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    Ok(bytes)
}

fn resample_to_16khz(samples: &[i16], source_rate: u32) -> Vec<i16> {
    if samples.is_empty() || source_rate == 16_000 {
        return samples.to_vec();
    }

    let output_len = ((samples.len() as u64 * 16_000) / source_rate as u64) as usize;

    let mut output = Vec::with_capacity(output_len);

    for index in 0..output_len {
        let position = index as f64 * source_rate as f64 / 16_000.0;

        let left = position.floor() as usize;

        let right = (left + 1).min(samples.len() - 1);

        let fraction = position - left as f64;

        let value = samples[left] as f64 * (1.0 - fraction) + samples[right] as f64 * fraction;

        output.push(value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }

    output
}

fn write_pcm_wav(
    path: &str,
    pcm: &[u8],
    sample_rate: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if pcm.len() % 2 != 0 {
        return Err("PCM byte count must be even for 16-bit audio".into());
    }

    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;

    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;

    let block_align = channels * bits_per_sample / 8;

    let data_size = pcm.len() as u32;

    let riff_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + pcm.len());

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());

    wav.extend_from_slice(b"WAVE");

    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size

    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format

    wav.extend_from_slice(&channels.to_le_bytes());

    wav.extend_from_slice(&sample_rate.to_le_bytes());

    wav.extend_from_slice(&byte_rate.to_le_bytes());

    wav.extend_from_slice(&block_align.to_le_bytes());

    wav.extend_from_slice(&bits_per_sample.to_le_bytes());

    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());

    wav.extend_from_slice(pcm);

    std::fs::write(path, wav)?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| "failed to install rustls crypto provider")?;
    }

    println!(
        "GEMINI_API_KEY present: {}",
        std::env::var("GEMINI_API_KEY").is_ok()
    );

    let input_path = env::args()
        .nth(1)
        .ok_or("usage: cargo run -p voice-command --example gemini_audio -- <mp3-file>")?;

    println!("Loading: {}", input_path);

    let audio = decode_mp3(&input_path)?;

    println!("Decoded {} bytes of 16 kHz mono PCM.", audio.len());

    let provider = GeminiLiveProvider::from_env()?;

    let mut session = provider.connect().await?;

    println!("✓ Gemini Live session ready.");

    session.send_audio(AudioChunk { samples: audio }).await?;

    println!("→ Audio sent to Gemini.");

    let mut output = Vec::new();

    loop {
        let event = session.poll_event().await?;

        match event {
            VoiceEvent::AudioOut(chunk) => {
                println!("← Audio chunk: {} bytes", chunk.samples.len());

                output.extend(chunk.samples);
            }

            VoiceEvent::TurnComplete => {
                println!("✓ Turn complete.");

                break;
            }

            VoiceEvent::Error(error) => {
                return Err(format!("Gemini error: {error}").into());
            }

            _ => {}
        }
    }

    write_pcm_wav("gemini-output.wav", &output, 24_000)?;

    println!("✓ Wrote {} bytes to gemini-output.wav", output.len());

    Ok(())
}
