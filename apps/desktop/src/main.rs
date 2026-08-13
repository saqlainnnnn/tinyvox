use std::path::PathBuf;

use audio::CpalAudioRecorder;

use cleanup::{CleanupPipeline, GroqCleaner, LocalLlamaCleaner};

use stt_pulse::PulseClient;

use tinyvox_engine::{
    controller::TinyVoxController, dictionary::shared as shared_dictionary,
    dictionary_store::DictionaryStore, last_dictation::shared as shared_last_dictation,
    state::AppState, stats::DayKey, stats_store::StatsStore, tool_registry::ToolRegistry,
};

use voice_command::{GeminiLiveProvider, VoiceAgent, VoiceAgentState};

use win::{
    HotkeyEvent, OverlayState, WindowsForeground, WindowsHotkey, WindowsOverlay,
    WindowsTextInjector,
};

fn current_day() -> Result<DayKey, Box<dyn std::error::Error + Send + Sync>> {
    let duration = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;

    let days = duration.as_secs() / 86_400;

    let mut year = 1970u16;

    let mut remaining_days = days;

    loop {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);

        let days_in_year = if leap { 366 } else { 365 };

        if remaining_days < days_in_year {
            break;
        }

        remaining_days -= days_in_year;

        year += 1;
    }

    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);

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

        remaining_days -= days_in_month;

        month += 1;
    }

    Ok(DayKey::new(year, month, remaining_days as u8 + 1))
}

/*
 * Wait for the next VoiceAgent state change.
 *
 * Returning Pending when voice mode is disabled lets the main
 * Tokio event loop continue waiting on the hotkey channel without
 * spawning another task or requiring WindowsOverlay::Clone.
 */
async fn wait_for_voice_state(
    receiver: &mut Option<tokio::sync::watch::Receiver<VoiceAgentState>>,
) -> Option<VoiceAgentState> {
    match receiver {
        Some(receiver) => match receiver.changed().await {
            Ok(()) => Some(*receiver.borrow()),

            Err(_) => None,
        },

        None => std::future::pending::<Option<VoiceAgentState>>().await,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();

    // ---------------------------------------------------------
    // rustls
    // ---------------------------------------------------------

    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| "failed to install rustls crypto provider")?;
    }

    println!("TinyVox");
    println!("=======");
    println!("Hold F9 to record.");
    println!("Press F10 to toggle voice-command mode.");
    println!("Press Ctrl+C to exit.\n");

    // ---------------------------------------------------------
    // Platform / hotkey
    // ---------------------------------------------------------

    let hotkey = WindowsHotkey::new()?;

    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel();

    /*
     * WindowsHotkey::recv() blocks on a std channel.
     *
     * Keep that blocking operation on its own OS thread and
     * forward semantic hotkey events into the async runtime.
     */
    std::thread::spawn(move || {
        loop {
            match hotkey.recv() {
                Ok(event) => {
                    if hotkey_tx.send(event).is_err() {
                        break;
                    }
                }

                Err(error) => {
                    eprintln!("⚠ Hotkey thread error: {error}");

                    break;
                }
            }
        }
    });

    let foreground = WindowsForeground::new();

    // ---------------------------------------------------------
    // Audio / STT
    // ---------------------------------------------------------

    let recorder = CpalAudioRecorder::new()?;

    let speech_to_text = PulseClient::from_env()?;

    // ---------------------------------------------------------
    // Cleanup
    // ---------------------------------------------------------

    let primary = GroqCleaner::from_env()?;

    let fallback = LocalLlamaCleaner::new();

    let cleaner = CleanupPipeline::new(primary, fallback);

    // ---------------------------------------------------------
    // Injection / overlay
    // ---------------------------------------------------------

    let injector = WindowsTextInjector::new();

    let overlay = WindowsOverlay::new()?;

    // ---------------------------------------------------------
    // Application data directory
    // ---------------------------------------------------------

    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA environment variable is not available")?
        .join("TinyVox");

    std::fs::create_dir_all(&app_data)?;

    // ---------------------------------------------------------
    // Dictionary
    // ---------------------------------------------------------

    let dictionary_path = app_data.join("dictionary.json");

    let dictionary_store = DictionaryStore::new(dictionary_path);

    let dictionary = dictionary_store.load()?;

    let shared_dictionary = shared_dictionary();

    {
        let mut target = shared_dictionary.write().unwrap();

        *target = dictionary;
    }

    // ---------------------------------------------------------
    // Last dictation
    // ---------------------------------------------------------

    let shared_last_dictation = shared_last_dictation();

    // ---------------------------------------------------------
    // Stats
    // ---------------------------------------------------------

    let stats_path = app_data.join("stats.json");

    let stats_store = StatsStore::new(stats_path);

    let stats = stats_store.load()?;

    println!(
        "📊 Loaded stats: {} words, {} dictations.",
        stats.total_words(),
        stats.total_dictations()
    );

    // ---------------------------------------------------------
    // Dictation controller
    // ---------------------------------------------------------

    let mut controller = TinyVoxController::new_with_last_dictation(
        recorder,
        speech_to_text,
        shared_dictionary.clone(),
        cleaner,
        injector,
        stats,
        shared_last_dictation.clone(),
    );

    // ---------------------------------------------------------
    // Voice agent
    // ---------------------------------------------------------

    let mut voice_agent: Option<VoiceAgent> = None;

    /*
     * This is consumed by the same async main loop.
     *
     * None = no voice mode.
     * Some(receiver) = voice agent is active.
     */
    let mut voice_state_rx: Option<tokio::sync::watch::Receiver<VoiceAgentState>> = None;

    // ---------------------------------------------------------
    // Dictation foreground target
    // ---------------------------------------------------------

    let mut target = None;

    // ---------------------------------------------------------
    // Main event loop
    // ---------------------------------------------------------

    loop {
        tokio::select! {
            /*
             * =================================================
             * HOTKEY EVENTS
             * =================================================
             */
            Some(event) =
                hotkey_rx.recv()
            => {
                match event {
                    // =========================================
                    // F9 PRESSED
                    // =========================================

                    HotkeyEvent::Pressed => {
                        /*
                         * Voice-command owns the microphone.
                         */
                        if voice_agent
                            .as_ref()
                            .is_some_and(
                                VoiceAgent::is_running,
                            )
                        {
                            println!(
                                "⚠ Voice-command mode is active."
                            );

                            overlay.set_state(
                                OverlayState::Busy,
                            );

                            overlay.show();

                            continue;
                        }

                        /*
                         * TinyVox cannot start another
                         * recording while the previous one
                         * is processing.
                         */
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

                        let window =
                            foreground.get()?;

                        println!(
                            "🎯 Target: {}",
                            window.process_name
                        );

                        target =
                            Some(window);

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

                    // =========================================
                    // F9 RELEASED
                    // =========================================

                    HotkeyEvent::Released => {
                        /*
                         * Ignore releases that aren't associated
                         * with an active dictation.
                         */
                        if controller.state()
                            != AppState::Recording
                        {
                            continue;
                        }

                        println!(
                            "🧠 Transcribing..."
                        );

                        let day =
                            current_day()?;

                        controller
                            .stop_recording(
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
                            )
                            .await?;

                        // -----------------------------
                        // Persist stats
                        // -----------------------------

                        if let Err(
                            error,
                        ) =
                            stats_store.save(
                                controller.stats(),
                            )
                        {
                            eprintln!(
                                "⚠ Failed to save stats: {error}"
                            );
                        }

                        // -----------------------------
                        // Persist dictionary
                        // -----------------------------

                        if let Ok(
                            dictionary,
                        ) =
                            shared_dictionary.read()
                        {
                            if let Err(
                                error,
                            ) =
                                dictionary_store
                                    .save(
                                        &dictionary,
                                    )
                            {
                                eprintln!(
                                    "⚠ Failed to save dictionary: {error}"
                                );
                            }
                        }

                        // -----------------------------
                        // Report result
                        // -----------------------------

                        if let Some(
                            window,
                        ) =
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

                    // =========================================
                    // F10 TOGGLE
                    // =========================================

                    HotkeyEvent::VoiceCommandToggled => {
                        /*
                         * -----------------------------------------
                         * VOICE MODE ON → STOP
                         * -----------------------------------------
                         */
                        if let Some(
                            mut agent,
                        ) =
                            voice_agent.take()
                        {
                            println!(
                                "🛑 Stopping voice-command mode..."
                            );

                            if let Err(
                                error,
                            ) =
                                agent
                                    .stop()
                                    .await
                            {
                                eprintln!(
                                    "⚠ Failed to stop voice agent: {error}"
                                );
                            }

                            /*
                             * Stop listening to the old agent's
                             * watch channel.
                             */
                            voice_state_rx =
                                None;

                            overlay.hide();

                            println!(
                                "✓ Voice-command mode disabled."
                            );

                            continue;
                        }

                        /*
                         * -----------------------------------------
                         * COLLISION CHECK
                         * -----------------------------------------
                         */
                        if controller
                            .state()
                            .is_busy()
                        {
                            println!(
                                "⚠ TinyVox is busy; cannot start voice-command mode."
                            );

                            overlay.set_state(
                                OverlayState::Busy,
                            );

                            overlay.show();

                            continue;
                        }

                        /*
                         * -----------------------------------------
                         * START VOICE MODE
                         * -----------------------------------------
                         */

                        println!(
                            "🎧 Starting voice-command mode..."
                        );

                        let provider =
                            match GeminiLiveProvider
                                ::from_env()
                            {
                                Ok(provider) => {
                                    provider
                                }

                                Err(error) => {
                                    eprintln!(
                                        "⚠ Gemini configuration error: {error}"
                                    );

                                    continue;
                                }
                            };

                        /*
                         * Same shared state as the dictation
                         * controller.
                         */
                        let tool_registry =
                            ToolRegistry::new(
                                shared_dictionary
                                    .clone(),
                                shared_last_dictation
                                    .clone(),
                            );

                        let mut agent =
                            VoiceAgent::new(
                                provider,
                                tool_registry,
                            );

                        /*
                         * Subscribe BEFORE start() so we never
                         * miss the initial Listening state.
                         */
                        let state_receiver =
                            agent.subscribe_state();

                        match agent.start().await {
                            Ok(()) => {
                                /*
                                 * The watch receiver remains
                                 * owned by this main event loop.
                                 */
                                voice_state_rx =
                                    Some(
                                        state_receiver,
                                    );

                                voice_agent =
                                    Some(agent);

                                /*
                                 * The agent starts in Listening.
                                 * Update immediately instead of
                                 * waiting for a later state change.
                                 */
                                apply_voice_overlay_state(
                                    VoiceAgentState::Listening,
                                    &overlay,
                                );

                                println!(
                                    "✓ Voice-command mode enabled."
                                );
                            }

                            Err(error) => {
                                eprintln!(
                                    "⚠ Failed to start voice agent: {error}"
                                );

                                /*
                                 * Don't leave stale state around
                                 * if startup failed.
                                 */
                                voice_state_rx =
                                    None;
                            }
                        }
                    }
                }
            }

            /*
             * =================================================
             * VOICE AGENT STATE
             * =================================================
             *
             * This fires whenever VoiceAgent publishes a new
             * Listening / Thinking / Speaking / BargeIn / Stopped
             * state.
             */
            state =
                wait_for_voice_state(
                    &mut voice_state_rx,
                )
                => {
                if let Some(
                    state,
                ) =
                    state
                {
                    /*
                     * Ignore stale state events after F10 has
                     * already turned the mode off.
                     */
                    if voice_agent
                        .as_ref()
                        .is_some_and(
                            VoiceAgent::is_running,
                        )
                    {
                        apply_voice_overlay_state(
                            state,
                            &overlay,
                        );
                    }
                }
            }
        }
    }
}

// =============================================================
// Voice overlay mapping
// =============================================================

fn apply_voice_overlay_state(state: VoiceAgentState, overlay: &WindowsOverlay) {
    match state {
        VoiceAgentState::Listening => {
            /*
             * Your current OverlayState API may not yet expose
             * dedicated Listening/Speaking variants. Until those
             * are added, Busy is the safest non-recording state
             * rather than falsely showing "Recording".
             */
            overlay.set_state(OverlayState::Busy);

            overlay.show();
        }

        VoiceAgentState::Thinking => {
            overlay.set_state(OverlayState::Transcribing);

            overlay.show();
        }

        VoiceAgentState::Speaking => {
            /*
             * If OverlayState::Speaking exists in the win crate,
             * replace Busy with:
             *
             *     OverlayState::Speaking
             *
             * For the current enum, Busy avoids pretending that
             * Gemini audio output is microphone recording.
             */
            overlay.set_state(OverlayState::Busy);

            overlay.show();
        }

        VoiceAgentState::BargeIn => {
            /*
             * Barge-in is transient. Once playback has been
             * cleared, VoiceAgent immediately publishes Listening.
             */
            overlay.set_state(OverlayState::Busy);

            overlay.show();
        }

        VoiceAgentState::Stopped => {
            overlay.hide();
        }
    }
}
