use audio::CpalAudioRecorder;
use tinyvox_engine::controller::TinyVoxController;
use win::{HotkeyEvent, WindowsHotkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TinyVox");
    println!("=======");
    println!("Hold F9 to record.");
    println!("Release F9 to stop.");
    println!("Press Ctrl+C to exit.\n");

    let hotkey = WindowsHotkey::new()?;
    let recorder = CpalAudioRecorder::new()?;

    let mut controller = TinyVoxController::new(recorder);

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                controller.start_recording()?;

                println!("🎙️ Recording...");
            }

            HotkeyEvent::Released => {
                let audio = controller.stop_recording()?;

                let duration =
                    audio.samples.len() as f32 / audio.sample_rate as f32;

                println!(
                    "✓ Captured {} samples @ {} Hz ({:.2}s)",
                    audio.samples.len(),
                    audio.sample_rate,
                    duration
                );
            }
        }
    }
}