use crate::{
    event::AppEvent,
    ports::{AudioRecorder, SpeechToText, Transcript},
    state::AppState,
};

#[derive(Debug)]
pub enum ControllerError<RE, SE> {
    InvalidTransition {
        state: AppState,
        event: AppEvent,
    },
    Recorder(RE),
    SpeechToText(SE),
}

impl<RE: std::fmt::Display, SE: std::fmt::Display> std::fmt::Display
    for ControllerError<RE, SE>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
        }
    }
}

impl<RE, SE> std::error::Error for ControllerError<RE, SE>
where
    RE: std::error::Error + 'static,
    SE: std::error::Error + 'static,
{
}

pub struct TinyVoxController<R, S>
where
    R: AudioRecorder,
    S: SpeechToText,
{
    state: AppState,
    recorder: R,
    speech_to_text: S,
}

impl<R, S> TinyVoxController<R, S>
where
    R: AudioRecorder,
    S: SpeechToText,
{
    pub fn new(
        recorder: R,
        speech_to_text: S,
    ) -> Self {
        Self {
            state: AppState::Idle,
            recorder,
            speech_to_text,
        }
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn start_recording(
        &mut self,
    ) -> Result<(), ControllerError<R::Error, S::Error>> {
        let event = AppEvent::RecordingStarted;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| ControllerError::InvalidTransition {
                state: self.state,
                event: event.clone(),
            })?;

        self.recorder
            .start()
            .map_err(ControllerError::Recorder)?;

        self.state = next_state;

        Ok(())
    }

    pub async fn stop_recording(
        &mut self,
    ) -> Result<Transcript, ControllerError<R::Error, S::Error>> {
        let event = AppEvent::RecordingStopped;

        let next_state = self
            .state
            .transition(&event)
            .ok_or_else(|| ControllerError::InvalidTransition {
                state: self.state,
                event: event.clone(),
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
            .ok_or_else(|| ControllerError::InvalidTransition {
                state: self.state,
                event: event.clone(),
            })?;

        self.state = next_state;

        Ok(transcript)
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
            assert_eq!(audio.sample_rate, 16_000);

            Ok(Transcript {
                text: "hello from TinyVox".to_string(),
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

        let transcript =
            controller.stop_recording().await.unwrap();

        assert_eq!(
            controller.state(),
            AppState::Cleaning
        );

        assert_eq!(
            transcript.text,
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