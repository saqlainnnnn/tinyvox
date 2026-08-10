use audio::CpalAudioRecorder;
use stt_pulse::PulseClient;
use tinyvox_engine::controller::TinyVoxController;
use win::{HotkeyEvent, WindowsHotkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("TinyVox");
    println!("=======");
    println!("Hold F9 to record.");
    println!("Release F9 to stop.");
    println!("Press Ctrl+C to exit.\n");

    let hotkey = WindowsHotkey::new()?;
    let recorder = CpalAudioRecorder::new()?;
    let pulse = PulseClient::from_env()?;

    let mut controller =
        TinyVoxController::new(recorder, pulse);

    let runtime = tokio::runtime::Runtime::new()?;

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                controller.start_recording()?;

                println!("🎙️ Recording...");
            }

            HotkeyEvent::Released => {
                println!("🧠 Transcribing...");

                let transcript = runtime.block_on(
                    controller.stop_recording()
                )?;

                println!("✓ Transcript:");
                println!("  {}", transcript.text);
            }
        }
    }
}