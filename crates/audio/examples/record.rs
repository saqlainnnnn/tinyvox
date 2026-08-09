use std::io::{self, Write};

use audio::CpalAudioRecorder;
use tinyvox_engine::ports::AudioRecorder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = CpalAudioRecorder::new()?;

    println!("Starting recording...");
    recorder.start()?;

    print!("Speak, then press ENTER to stop: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let audio = recorder.stop()?;

    println!(
        "Captured {} samples at {} Hz",
        audio.samples.len(),
        audio.sample_rate
    );

    println!(
        "Duration: {:.2}s",
        audio.samples.len() as f32 / audio.sample_rate as f32
    );

    Ok(())
}