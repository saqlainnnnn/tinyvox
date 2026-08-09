use crate::{
    event::AppEvent,
    ports::{AudioBuffer, AudioRecorder},
    state::AppState,
};

#[derive(Debug)]
pub enum ControllerError<E> {
    InvalidTransition {
        state: AppState,
        event: AppEvent,
    },
    Recorder(E),
}

impl<E: std::fmt::Display> std::fmt::Display for ControllerError<E> {
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
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ControllerError<E> {}

pub struct TinyVoxController<R>
where
    R: AudioRecorder,
{
    state: AppState,
    recorder: R,
}

impl<R> TinyVoxController<R>
where
    R: AudioRecorder,
{
    pub fn new(recorder: R) -> Self {
        Self {
            state: AppState::Idle,
            recorder,
        }
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn start_recording(
        &mut self,
    ) -> Result<(), ControllerError<R::Error>> {
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

    pub fn stop_recording(
        &mut self,
    ) -> Result<AudioBuffer, ControllerError<R::Error>> {
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

        Ok(audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRecorder {
        recording: bool,
    }

    impl AudioRecorder for FakeRecorder {
        type Error = &'static str;

        fn start(&mut self) -> Result<(), Self::Error> {
            self.recording = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<AudioBuffer, Self::Error> {
            self.recording = false;

            Ok(AudioBuffer {
                samples: vec![0.0; 16_000],
                sample_rate: 16_000,
            })
        }
    }

    #[test]
    fn recording_lifecycle_is_managed_by_controller() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller = TinyVoxController::new(recorder);

        assert_eq!(controller.state(), AppState::Idle);

        controller.start_recording().unwrap();

        assert_eq!(controller.state(), AppState::Recording);

        let audio = controller.stop_recording().unwrap();

        assert_eq!(controller.state(), AppState::Transcribing);
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.samples.len(), 16_000);
    }

    #[test]
    fn cannot_start_twice() {
        let recorder = FakeRecorder {
            recording: false,
        };

        let mut controller = TinyVoxController::new(recorder);

        controller.start_recording().unwrap();

        let result = controller.start_recording();

        assert!(matches!(
            result,
            Err(ControllerError::InvalidTransition {
                state: AppState::Recording,
                event: AppEvent::RecordingStarted,
            })
        ));
    }
}