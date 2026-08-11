use std::path::PathBuf;

use audio::CpalAudioRecorder;
use cleanup::{
    CleanupPipeline,
    GroqCleaner,
    LocalLlamaCleaner,
};
use stt_pulse::PulseClient;
use tinyvox_engine::{
    controller::TinyVoxController,
    dictionary_store::DictionaryStore,
    state::AppState,
};
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

    let dictionary_path = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA environment variable is not available")?
        .join("TinyVox")
        .join("dictionary.json");

    let dictionary_store =
        DictionaryStore::new(dictionary_path);

    let dictionary =
        dictionary_store.load()?;

    let mut controller =
        TinyVoxController::new(
            recorder,
            speech_to_text,
            dictionary,
            cleaner,
            injector,
        );

    let runtime =
        tokio::runtime::Runtime::new()?;

    let mut target = None;

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                if controller.state().is_busy() {
                    println!("⚠ TinyVox is busy.");

                    overlay.set_state(
                        OverlayState::Busy,
                    );

                    overlay.show();

                    continue;
                }

                let window =
                    foreground.get()?;

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

                println!(
                    "🎙️ Recording..."
                );
            }

            HotkeyEvent::Released => {
                println!(
                    "🧠 Transcribing..."
                );

                runtime.block_on(
                    controller.stop_recording(
                        |state| {
                            match state {
                                AppState::Transcribing => {
                                    overlay.set_state(
                                        OverlayState::Transcribing,
                                    );
                                }

                                AppState::Cleaning => {
                                    overlay.set_state(
                                        OverlayState::Cleaning,
                                    );
                                }

                                AppState::Injecting => {
                                    overlay.set_state(
                                        OverlayState::Injecting,
                                    );
                                }

                                AppState::Idle => {
                                    overlay.hide();
                                }

                                _ => {}
                            }
                        },
                    ),
                )?;

                if let Some(window) =
                    target.take()
                {
                    println!(
                        "✓ Injected into {}.",
                        window.process_name
                    );
                } else {
                    println!(
                        "✓ Injected."
                    );
                }
            }
        }
    }
}