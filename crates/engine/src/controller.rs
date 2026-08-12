use crate::{
    dictionary::Dictionary,
    event::AppEvent,
    ports::{
        AudioRecorder,
        CleanedText,
        SpeechToText,
        TextCleaner,
        TextInjector,
        Transcript,
    },
    state::AppState,
    stats::{DayKey, DictationStats},
};

#[derive(Debug)]
pub enum ControllerError<RE, SE, CE, IE> {
    InvalidTransition {
        state: AppState,
        event: AppEvent,
    },
    Recorder(RE),
    SpeechToText(SE),
    Cleaner(CE),
    Injector(IE),
}

impl<
    RE: std::fmt::Display,
    SE: std::fmt::Display,
    CE: std::fmt::Display,
    IE: std::fmt::Display,
> std::fmt::Display for ControllerError<RE, SE, CE, IE> {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::InvalidTransition {
                state,
                event,
            } => {
                write!(
                    f,
                    "invalid transition from {state:?} using {event:?}"
                )
            }

            Self::Recorder(error) => {
                write!(
                    f,
                    "audio recorder error: {error}"
                )
            }

            Self::SpeechToText(error) => {
                write!(
                    f,
                    "speech-to-text error: {error}"
                )
            }

            Self::Cleaner(error) => {
                write!(
                    f,
                    "text cleanup error: {error}"
                )
            }

            Self::Injector(error) => {
                write!(
                    f,
                    "text injection error: {error}"
                )
            }
        }
    }
}

impl<RE, SE, CE, IE> std::error::Error
    for ControllerError<RE, SE, CE, IE>
where
    RE: std::error::Error + 'static,
    SE: std::error::Error + 'static,
    CE: std::error::Error + 'static,
    IE: std::error::Error + 'static,
{
}

pub struct TinyVoxController<R, S, C, I>
where
    R: AudioRecorder,
    S: SpeechToText,
    C: TextCleaner,
    I: TextInjector,
{
    state: AppState,
    recorder: R,
    speech_to_text: S,
    dictionary: Dictionary,
    cleaner: C,
    injector: I,
    stats: DictationStats,
}

impl<R, S, C, I> TinyVoxController<R, S, C, I>
where
    R: AudioRecorder,
    S: SpeechToText,
    C: TextCleaner,
    I: TextInjector,
{
    pub fn new(
        recorder: R,
        speech_to_text: S,
        dictionary: Dictionary,
        cleaner: C,
        injector: I,
        stats: DictationStats,
    ) -> Self {
        Self {
            state: AppState::Idle,
            recorder,
            speech_to_text,
            dictionary,
            cleaner,
            injector,
            stats,
        }
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn stats(&self) -> &DictationStats {
        &self.stats
    }

    pub fn stats_mut(&mut self) -> &mut DictationStats {
        &mut self.stats
    }

    pub fn start_recording(
        &mut self,
    ) -> Result<
        (),
        ControllerError<
            R::Error,
            S::Error,
            C::Error,
            I::Error,
        >,
    > {
        let event =
            AppEvent::RecordingStarted;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| {
                ControllerError::InvalidTransition {
                    state: self.state,
                    event: event.clone(),
                }
            })?;

        self.injector
            .prepare()
            .map_err(ControllerError::Injector)?;

        self.recorder
            .start()
            .map_err(ControllerError::Recorder)?;

        self.state = next_state;

        Ok(())
    }

    pub async fn stop_recording<F>(
        &mut self,
        day: DayKey,
        on_state_change: F,
    ) -> Result<
        (),
        ControllerError<
            R::Error,
            S::Error,
            C::Error,
            I::Error,
        >,
    >
    where
        F: Fn(AppState),
    {
        // Recording → Transcribing
        let event =
            AppEvent::RecordingStopped;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| {
                ControllerError::InvalidTransition {
                    state: self.state,
                    event: event.clone(),
                }
            })?;

        let audio = self
            .recorder
            .stop()
            .map_err(ControllerError::Recorder)?;

        /*
         * Recording duration is derived from the
         * captured audio itself.
         *
         * This deliberately measures recording time,
         * not transcription/cleanup latency.
         */
        let recording_ms =
            if audio.sample_rate == 0 {
                0
            } else {
                (
                    audio.samples.len() as u64
                    * 1_000
                ) / audio.sample_rate as u64
            };

        self.state = next_state;
        on_state_change(self.state);

        // Transcribing
        let transcript = self
            .speech_to_text
            .transcribe(&audio)
            .await
            .map_err(ControllerError::SpeechToText)?;

        // Apply dictionary corrections
        let corrected_text =
            self.dictionary
                .apply(&transcript.text);

        let transcript = Transcript {
            text: corrected_text,
        };

        // Transcribing → Cleaning
        let event =
            AppEvent::TranscriptionCompleted;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| {
                ControllerError::InvalidTransition {
                    state: self.state,
                    event: event.clone(),
                }
            })?;

        self.state = next_state;
        on_state_change(self.state);

        // Cleaning
        let cleaned_text = self
            .cleaner
            .clean(&transcript)
            .await
            .map_err(ControllerError::Cleaner)?;

        // Cleaning → Injecting
        let event =
            AppEvent::CleanupCompleted;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| {
                ControllerError::InvalidTransition {
                    state: self.state,
                    event: event.clone(),
                }
            })?;

        self.state = next_state;
        on_state_change(self.state);

        // Injecting
        self.injector
            .inject(&cleaned_text)
            .map_err(ControllerError::Injector)?;

        /*
         * Only record statistics after injection
         * succeeds.
         *
         * Stats therefore cannot interfere with the
         * actual dictation pipeline.
         */
        let word_count =
            cleaned_text
                .text
                .split_whitespace()
                .count() as u32;

        self.stats.record(
            day,
            word_count,
            recording_ms,
        );

        // Injecting → Idle
        let event =
            AppEvent::InjectionCompleted;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| {
                ControllerError::InvalidTransition {
                    state: self.state,
                    event: event.clone(),
                }
            })?;

        self.state = next_state;
        on_state_change(self.state);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        dictionary::EntrySource,
        ports::{
            AudioBuffer,
            CleanedText,
            Transcript,
        },
    };

    use std::sync::{
        Arc,
        Mutex,
    };

    struct FakeRecorder {
        recording: bool,
    }

    impl AudioRecorder for FakeRecorder {
        type Error = &'static str;

        fn start(
            &mut self,
        ) -> Result<(), Self::Error> {
            self.recording = true;
            Ok(())
        }

        fn stop(
            &mut self,
        ) -> Result<AudioBuffer, Self::Error> {
            self.recording = false;

            Ok(AudioBuffer {
                samples: vec![0.0; 16_000],
                sample_rate: 16_000,
            })
        }
    }

    struct FakeSpeechToText;

    impl SpeechToText for FakeSpeechToText {
        type Error = &'static str;

        async fn transcribe(
            &self,
            audio: &AudioBuffer,
        ) -> Result<Transcript, Self::Error> {
            assert_eq!(
                audio.sample_rate,
                16_000
            );

            Ok(Transcript {
                text:
                    "  hello from TinyVox  "
                        .to_string(),
            })
        }
    }

    struct DictionarySpeechToText;

    impl SpeechToText
        for DictionarySpeechToText
    {
        type Error = &'static str;

        async fn transcribe(
            &self,
            _audio: &AudioBuffer,
        ) -> Result<Transcript, Self::Error> {
            Ok(Transcript {
                text:
                    "I use Kubernets every day."
                        .to_string(),
            })
        }
    }

    struct FakeCleaner {
        received:
            Arc<Mutex<Option<String>>>,
    }

    impl TextCleaner for FakeCleaner {
        type Error = &'static str;

        fn clean(
            &self,
            transcript: &Transcript,
        ) -> impl std::future::Future<
            Output =
                Result<
                    CleanedText,
                    Self::Error,
                >,
        > + Send {
            let received =
                self.received.clone();

            let text =
                transcript.text.clone();

            async move {
                *received
                    .lock()
                    .unwrap() =
                    Some(text.clone());

                Ok(CleanedText {
                    text: text
                        .trim()
                        .to_string(),
                })
            }
        }
    }

    struct FakeInjector;

    impl TextInjector for FakeInjector {
        type Error = std::io::Error;

        fn prepare(
            &mut self,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn inject(
            &self,
            _text: &CleanedText,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn test_day() -> DayKey {
        DayKey::new(
            2026,
            8,
            12,
        )
    }

    #[tokio::test]
    async fn recording_lifecycle_is_managed_by_controller() {
        let recorder =
            FakeRecorder {
                recording: false,
            };

        let mut controller =
            TinyVoxController::new(
                recorder,
                FakeSpeechToText,
                Dictionary::new(),
                FakeCleaner {
                    received: Arc::new(
                        Mutex::new(None),
                    ),
                },
                FakeInjector,
                DictationStats::new(),
            );

        assert_eq!(
            controller.state(),
            AppState::Idle
        );

        controller
            .start_recording()
            .unwrap();

        assert_eq!(
            controller.state(),
            AppState::Recording
        );

        controller
            .stop_recording(
                test_day(),
                |_| {},
            )
            .await
            .unwrap();

        assert_eq!(
            controller.state(),
            AppState::Idle
        );
    }

    #[tokio::test]
    async fn cannot_start_twice() {
        let recorder =
            FakeRecorder {
                recording: false,
            };

        let mut controller =
            TinyVoxController::new(
                recorder,
                FakeSpeechToText,
                Dictionary::new(),
                FakeCleaner {
                    received: Arc::new(
                        Mutex::new(None),
                    ),
                },
                FakeInjector,
                DictationStats::new(),
            );

        controller
            .start_recording()
            .unwrap();

        let result =
            controller.start_recording();

        assert!(matches!(
            result,
            Err(
                ControllerError::InvalidTransition {
                    state: AppState::Recording,
                    event:
                        AppEvent::RecordingStarted,
                }
            )
        ));
    }

    #[tokio::test]
    async fn dictionary_correction_reaches_cleaner() {
        let recorder =
            FakeRecorder {
                recording: false,
            };

        let received =
            Arc::new(Mutex::new(None));

        let cleaner =
            FakeCleaner {
                received:
                    received.clone(),
            };

        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "kubernets",
            "Kubernetes",
            EntrySource::Manual,
        );

        let mut controller =
            TinyVoxController::new(
                recorder,
                DictionarySpeechToText,
                dictionary,
                cleaner,
                FakeInjector,
                DictationStats::new(),
            );

        controller
            .start_recording()
            .unwrap();

        controller
            .stop_recording(
                test_day(),
                |_| {},
            )
            .await
            .unwrap();

        assert_eq!(
            received
                .lock()
                .unwrap()
                .as_deref(),
            Some(
                "I use Kubernetes every day."
            )
        );
    }

    #[tokio::test]
    async fn successful_injection_records_stats() {
        let recorder =
            FakeRecorder {
                recording: false,
            };

        let mut controller =
            TinyVoxController::new(
                recorder,
                FakeSpeechToText,
                Dictionary::new(),
                FakeCleaner {
                    received: Arc::new(
                        Mutex::new(None),
                    ),
                },
                FakeInjector,
                DictationStats::new(),
            );

        controller
            .start_recording()
            .unwrap();

        controller
            .stop_recording(
                test_day(),
                |_| {},
            )
            .await
            .unwrap();

        assert_eq!(
            controller
                .stats()
                .total_dictations(),
            1
        );

        assert_eq!(
            controller
                .stats()
                .total_words(),
            3
        );

        assert_eq!(
            controller
                .stats()
                .today(test_day())
                .dictations,
            1
        );

        assert_eq!(
            controller
                .stats()
                .today(test_day())
                .words,
            3
        );
    }
}