use crate::{
    event::AppEvent,
    ports::{
        CleanedText,
        AudioRecorder,
        SpeechToText,
        TextCleaner,
        Transcript,
    },
    state::AppState,
};

#[derive(Debug)]
pub enum ControllerError<RE, SE, CE> {
    InvalidTransition {
        state: AppState,
        event: AppEvent,
    },
    Recorder(RE),
    SpeechToText(SE),
    Cleaner(CE),
}

impl<
        RE: std::fmt::Display,
        SE: std::fmt::Display,
        CE: std::fmt::Display,
    > std::fmt::Display for ControllerError<RE, SE, CE>
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
        }
    }
}

impl<RE, SE, CE> std::error::Error
    for ControllerError<RE, SE, CE>
where
    RE: std::error::Error + 'static,
    SE: std::error::Error + 'static,
    CE: std::error::Error + 'static,
{
}

pub struct TinyVoxController<R, S, C>
where
    R: AudioRecorder,
    S: SpeechToText,
    C: TextCleaner,
{
    state: AppState,
    recorder: R,
    speech_to_text: S,
    cleaner: C,
}

impl<R, S, C> TinyVoxController<R, S, C>
where
    R: AudioRecorder,
    S: SpeechToText,
    C: TextCleaner,
{
    pub fn new(
        recorder: R,
        speech_to_text: S,
        cleaner: C,
    ) -> Self {
        Self {
            state: AppState::Idle,
            recorder,
            speech_to_text,
            cleaner,
        }
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn start_recording(
        &mut self,
    ) -> Result<
        (),
        ControllerError<R::Error, S::Error, C::Error>,
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

        self.recorder
            .start()
            .map_err(ControllerError::Recorder)?;

        self.state = next_state;

        Ok(())
    }

    pub async fn stop_recording(
        &mut self,
    ) -> Result<
        CleanedText,
        ControllerError<R::Error, S::Error, C::Error>,
    > {
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

        let transcript = self
            .speech_to_text
            .transcribe(&audio)
            .await
            .map_err(ControllerError::SpeechToText)?;

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

        let cleaned_text = self
            .cleaner
            .clean(&transcript)
            .map_err(ControllerError::Cleaner)?;

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

        Ok(cleaned_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::AudioBuffer;

    struct FakeRecorder {
        recording: bool,
    }

    impl AudioRecorder for FakeRecorder {
        type Error = &'static str;

        fn start(&mut self) -> Result<(), Self::Error> {
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
                text: "  hello from TinyVox  ".to_string(),
            })
        }
    }

    struct FakeCleaner;

    impl TextCleaner for FakeCleaner {
        type Error = &'static str;

        fn clean(
            &self,
            transcript: &Transcript,
        ) -> Result<CleanedText, Self::Error> {
            Ok(CleanedText {
                text: transcript.text.trim().to_string(),
            })
        }
    }

    #[tokio::test]
    async fn recording_lifecycle_is_managed_by_controller() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller = TinyVoxController::new(
            recorder,
            FakeSpeechToText,
            FakeCleaner,
        );

        assert_eq!(
            controller.state(),
            AppState::Idle
        );

        controller.start_recording().unwrap();

        assert_eq!(
            controller.state(),
            AppState::Recording
        );

        let cleaned_text =
            controller.stop_recording().await.unwrap();

        assert_eq!(
            controller.state(),
            AppState::Injecting
        );

        assert_eq!(
            cleaned_text.text,
            "hello from TinyVox"
        );
    }

    #[tokio::test]
    async fn cannot_start_twice() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller = TinyVoxController::new(
            recorder,
            FakeSpeechToText,
            FakeCleaner,
        );

        controller.start_recording().unwrap();

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