use audio::CpalAudioRecorder;
use cleanup::{
    CleanupPipeline,
    GroqCleaner,
    LocalLlamaCleaner,
};
use stt_pulse::PulseClient;
use tinyvox_engine::controller::TinyVoxController;
use win::{
    HotkeyEvent,
    WindowsHotkey,
    WindowsTextInjector,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("TinyVox");
    println!("=======");
    println!("Hold F9 to record.");
    println!("Release F9 to stop.");
    println!("Press Ctrl+C to exit.\n");

    let hotkey = WindowsHotkey::new()?;
    let recorder = CpalAudioRecorder::new()?;
    let speech_to_text = PulseClient::from_env()?;
    let injector = WindowsTextInjector::new();

    let primary = GroqCleaner::from_env()?;
    let fallback = LocalLlamaCleaner::new();

    let cleaner = CleanupPipeline::new(
        primary,
        fallback,
    );

    let mut controller =
        TinyVoxController::new(
            recorder,
            speech_to_text,
            cleaner,
            injector,
        );

    let runtime = tokio::runtime::Runtime::new()?;

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                controller.start_recording()?;

                println!("🎙️ Recording...");
            }

            HotkeyEvent::Released => {
                println!("🧠 Transcribing...");

                runtime.block_on(
                    controller.stop_recording(),
                )?;

                println!("✓ Injected.");
            }
        }
    }
}