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
    dictionary::{
        shared as shared_dictionary,
        Dictionary,
    },
    dictionary_store::DictionaryStore,
    state::AppState,
    stats::DayKey,
    stats_store::StatsStore,
};
use win::{
    HotkeyEvent,
    OverlayState,
    WindowsForeground,
    WindowsHotkey,
    WindowsOverlay,
    WindowsTextInjector,
};

fn current_day() -> Result<DayKey, Box<dyn std::error::Error>> {
    let duration =
        std::time::SystemTime::now()
            .duration_since(
                std::time::UNIX_EPOCH,
            )?;

    let days =
        duration.as_secs() / 86_400;

    let mut year = 1970u16;
    let mut remaining_days = days;

    loop {
        let leap =
            year % 4 == 0
                && (year % 100 != 0
                    || year % 400 == 0);

        let days_in_year =
            if leap {
                366
            } else {
                365
            };

        if remaining_days < days_in_year {
            break;
        }

        remaining_days -=
            days_in_year;

        year += 1;
    }

    let leap =
        year % 4 == 0
            && (year % 100 != 0
                || year % 400 == 0);

    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 1u8;

    for days_in_month in month_days {
        if remaining_days < days_in_month {
            break;
        }

        remaining_days -=
            days_in_month;

        month += 1;
    }

    Ok(DayKey::new(
        year,
        month,
        remaining_days as u8 + 1,
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("TinyVox");
    println!("=======");
    println!("Hold F9 to record.");
    println!("Release F9 to stop.");
    println!("Press Ctrl+C to exit.\n");

    // ---------------------------------------------------------
    // Platform
    // ---------------------------------------------------------

    let hotkey = WindowsHotkey::new()?;
    let foreground = WindowsForeground::new();

    // ---------------------------------------------------------
    // Audio / STT
    // ---------------------------------------------------------

    let recorder =
        CpalAudioRecorder::new()?;

    let speech_to_text =
        PulseClient::from_env()?;

    // ---------------------------------------------------------
    // Cleanup
    // ---------------------------------------------------------

    let primary =
        GroqCleaner::from_env()?;

    let fallback =
        LocalLlamaCleaner::new();

    let cleaner =
        CleanupPipeline::new(
            primary,
            fallback,
        );

    // ---------------------------------------------------------
    // Injection / overlay
    // ---------------------------------------------------------

    let injector =
        WindowsTextInjector::new();

    let overlay =
        WindowsOverlay::new()?;

    // ---------------------------------------------------------
    // Application data directory
    // ---------------------------------------------------------

    let app_data =
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or(
                "APPDATA environment variable is not available",
            )?
            .join("TinyVox");

    // ---------------------------------------------------------
    // Dictionary
    // ---------------------------------------------------------

    let dictionary_path =
        app_data.join("dictionary.json");

    let dictionary_store =
        DictionaryStore::new(
            dictionary_path,
        );

    let dictionary =
        dictionary_store.load()?;

    let shared_dictionary =
        shared_dictionary();

    *shared_dictionary
        .write()
        .unwrap() = dictionary;

    // ---------------------------------------------------------
    // Stats
    // ---------------------------------------------------------

    let stats_path =
        app_data.join("stats.json");

    let stats_store =
        StatsStore::new(stats_path);

    let stats =
        stats_store.load()?;

    println!(
        "📊 Loaded stats: {} words, {} dictations.",
        stats.total_words(),
        stats.total_dictations()
    );

    // ---------------------------------------------------------
    // Controller
    // ---------------------------------------------------------

    let mut controller =
        TinyVoxController::new(
            recorder,
            speech_to_text,
            shared_dictionary,
            cleaner,
            injector,
            stats,
        );

    let runtime =
        tokio::runtime::Runtime::new()?;

    let mut target = None;

    // ---------------------------------------------------------
    // Main event loop
    // ---------------------------------------------------------

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                // Prevent starting another recording while
                // TinyVox is processing the previous one.
                if controller
                    .state()
                    .is_busy()
                {
                    println!(
                        "⚠ TinyVox is busy."
                    );

                    overlay.set_state(
                        OverlayState::Busy,
                    );

                    overlay.show();

                    continue;
                }

                // Capture the foreground application
                // before recording begins.
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

                controller
                    .start_recording()?;

                println!(
                    "🎙️ Recording..."
                );
            }

            HotkeyEvent::Released => {
                println!(
                    "🧠 Transcribing..."
                );

                let day =
                    current_day()?;

                runtime.block_on(
                    controller.stop_recording(
                        day,
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

                // -------------------------------------------------
                // Persist stats after successful dictation.
                //
                // A stats failure must NOT break injection,
                // so saving errors are only logged.
                // -------------------------------------------------

                if let Err(error) =
                    stats_store.save(
                        controller.stats(),
                    )
                {
                    eprintln!(
                        "⚠ Failed to save stats: {error}"
                    );
                }

                // -------------------------------------------------
                // Report result
                // -------------------------------------------------

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

                println!(
                    "📊 Today: {} words | {} dictations | {} day streak",
                    controller
                        .stats()
                        .today(day)
                        .words,
                    controller
                        .stats()
                        .today(day)
                        .dictations,
                    controller
                        .stats()
                        .current_streak(day),
                );
            }
        }
    }
}