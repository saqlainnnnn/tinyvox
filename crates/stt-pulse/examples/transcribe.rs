use std::io::{self, Write};

use audio::CpalAudioRecorder;
use tinyvox_engine::ports::{AudioRecorder, SpeechToText};
use tokio::runtime::Runtime;
use stt_pulse::PulseClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("TinyVox — Pulse STT Test");
    println!("========================\n");

    let mut recorder = CpalAudioRecorder::new()?;

    print!("Press ENTER to start recording...");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    recorder.start()?;

    println!("🎙️ Recording...");
    println!("Press ENTER to stop.");

    input.clear();
    io::stdin().read_line(&mut input)?;

    let audio = recorder.stop()?;

    let duration =
        audio.samples.len() as f32 / audio.sample_rate as f32;

    println!(
        "✓ Captured {} samples @ {} Hz ({:.2}s)",
        audio.samples.len(),
        audio.sample_rate,
        duration
    );

    println!("\nTranscribing...");

    let pulse = PulseClient::from_env()?;

    let runtime = Runtime::new()?;

    let transcript = runtime.block_on(
        pulse.transcribe(&audio)
    )?;

    println!("\nTranscript:");
    println!("-----------");
    println!("{}", transcript.text);

    Ok(())
}