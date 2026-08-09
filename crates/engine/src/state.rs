use crate::event::AppEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Idle,
    Recording,
    Transcribing,
    Cleaning,
    Injecting,
}

impl AppState {
    pub fn is_busy(self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn transition(self, event: &AppEvent) -> Option<Self> {
        match (self, event) {
            (Self::Idle, AppEvent::RecordingStarted) => Some(Self::Recording),

            (Self::Recording, AppEvent::RecordingStopped) => {
                Some(Self::Transcribing)
            }

            (Self::Transcribing, AppEvent::TranscriptionCompleted) => {
                Some(Self::Cleaning)
            }

            (Self::Cleaning, AppEvent::CleanupCompleted) => {
                Some(Self::Injecting)
            }

            (Self::Injecting, AppEvent::InjectionCompleted) => {
                Some(Self::Idle)
            }

            (_, AppEvent::Failed) => Some(Self::Idle),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AppEvent;

    #[test]
    fn normal_dictation_flow() {
        let state = AppState::Idle;

        let state = state
            .transition(&AppEvent::RecordingStarted)
            .unwrap();
        assert_eq!(state, AppState::Recording);

        let state = state
            .transition(&AppEvent::RecordingStopped)
            .unwrap();
        assert_eq!(state, AppState::Transcribing);

        let state = state
            .transition(&AppEvent::TranscriptionCompleted)
            .unwrap();
        assert_eq!(state, AppState::Cleaning);

        let state = state
            .transition(&AppEvent::CleanupCompleted)
            .unwrap();
        assert_eq!(state, AppState::Injecting);

        let state = state
            .transition(&AppEvent::InjectionCompleted)
            .unwrap();
        assert_eq!(state, AppState::Idle);
    }

    #[test]
    fn invalid_transition_is_rejected() {
        assert_eq!(
            AppState::Idle.transition(&AppEvent::RecordingStopped),
            None
        );
    }

    #[test]
    fn failure_returns_to_idle() {
        assert_eq!(
            AppState::Transcribing.transition(&AppEvent::Failed),
            Some(AppState::Idle)
        );
    }
}