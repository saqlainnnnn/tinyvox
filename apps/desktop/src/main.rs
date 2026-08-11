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
    OverlayState,
    WindowsForeground,
    WindowsHotkey,
    WindowsOverlay,
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
    let foreground = WindowsForeground::new();

    let recorder = CpalAudioRecorder::new()?;
    let speech_to_text = PulseClient::from_env()?;

    let primary = GroqCleaner::from_env()?;
    let fallback = LocalLlamaCleaner::new();

    let cleaner = CleanupPipeline::new(
        primary,
        fallback,
    );

    let injector = WindowsTextInjector::new();

    let overlay = WindowsOverlay::new()?;

    let mut controller =
        TinyVoxController::new(
            recorder,
            speech_to_text,
            cleaner,
            injector,
        );

    let runtime = tokio::runtime::Runtime::new()?;

    let mut target = None;

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                let window = foreground.get()?;

                println!(
                    "🎯 Target: {}",
                    window.process_name
                );

                target = Some(window);

                overlay.set_state(
                    OverlayState::Recording,
                );
                overlay.show();

                controller.start_recording()?;

                println!("🎙️ Recording...");
            }

            HotkeyEvent::Released => {
                overlay.set_state(
                    OverlayState::Transcribing,
                );

                println!("🧠 Transcribing...");

                runtime.block_on(
                    controller.stop_recording(),
                )?;

                overlay.hide();

                if let Some(window) = target.take() {
                    println!(
                        "✓ Injected into {}.",
                        window.process_name
                    );
                } else {
                    println!("✓ Injected.");
                }
            }
        }
    }
}