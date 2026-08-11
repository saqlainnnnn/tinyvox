use crate::{
    event::AppEvent,
    ports::{
        AudioRecorder,
        CleanedText,
        SpeechToText,
        TextCleaner,
        TextInjector,
    },
    state::AppState,
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
> std::fmt::Display for ControllerError<RE, SE, CE, IE>
{
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { state, event } => {
                write!(
                    f,
                    "invalid transition from {state:?} using {event:?}"
                )
            }

            Self::Recorder(error) => {
                write!(f, "audio recorder error: {error}")
            }

            Self::SpeechToText(error) => {
                write!(f, "speech-to-text error: {error}")
            }

            Self::Cleaner(error) => {
                write!(f, "text cleanup error: {error}")
            }

            Self::Injector(error) => {
                write!(f, "text injection error: {error}")
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
    cleaner: C,
    injector: I,
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
        cleaner: C,
        injector: I,
    ) -> Self {
        Self {
            state: AppState::Idle,
            recorder,
            speech_to_text,
            cleaner,
            injector,
        }
    }

    pub fn state(&self) -> AppState {
        self.state
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
        let event = AppEvent::RecordingStarted;

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
        let event = AppEvent::RecordingStopped;

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

        self.state = next_state;
        on_state_change(self.state);

        // Transcribing
        let transcript = self
            .speech_to_text
            .transcribe(&audio)
            .await
            .map_err(ControllerError::SpeechToText)?;

        // Transcribing → Cleaning
        let event = AppEvent::TranscriptionCompleted;

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
        let event = AppEvent::CleanupCompleted;

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

        // Injecting → Idle
        let event = AppEvent::InjectionCompleted;

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
    use crate::ports::{
        AudioBuffer,
        Transcript,
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
                text: "  hello from TinyVox  "
                    .to_string(),
            })
        }
    }

    struct FakeCleaner;

    impl TextCleaner for FakeCleaner {
        type Error = &'static str;

        fn clean(
            &self,
            transcript: &Transcript,
        ) -> impl std::future::Future<
            Output = Result<
                CleanedText,
                Self::Error,
            >,
        > + Send {
            async move {
                Ok(CleanedText {
                    text: transcript
                        .text
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
            &self,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn inject(
            &self,
            text: &CleanedText,
        ) -> Result<(), Self::Error> {
            assert_eq!(
                text.text,
                "hello from TinyVox"
            );

            Ok(())
        }
    }

    #[tokio::test]
    async fn recording_lifecycle_is_managed_by_controller() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller =
            TinyVoxController::new(
                recorder,
                FakeSpeechToText,
                FakeCleaner,
                FakeInjector,
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
            .stop_recording(|_| {})
            .await
            .unwrap();

        assert_eq!(
            controller.state(),
            AppState::Idle
        );
    }

    #[tokio::test]
    async fn cannot_start_twice() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller =
            TinyVoxController::new(
                recorder,
                FakeSpeechToText,
                FakeCleaner,
                FakeInjector,
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
                    event: AppEvent::RecordingStarted,
                }
            )
        ));
    }
}