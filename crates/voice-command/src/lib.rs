pub use ports::{
    AudioChunk,
    ToolCall,
    VoiceProvider,
    VoiceSession,
};

pub mod ports;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    Disconnected,
    Connecting,
    Listening,
    UserTalking,
    Thinking,
    Speaking,
    BargeIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceEvent {
    ConnectRequested,
    Connected,
    ConnectionFailed,
    Disconnected,

    UserStartedTalking,
    UserStoppedTalking,

    ResponseStarted,
    ResponseCompleted,

    ToolCallReceived,
    ToolCallCompleted,

    BargeInDetected,
}

impl VoiceState {
    pub fn is_connected(self) -> bool {
        !matches!(
            self,
            Self::Disconnected
        )
    }

    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Connecting
                | Self::UserTalking
                | Self::Thinking
                | Self::Speaking
                | Self::BargeIn
        )
    }

    pub fn transition(
        self,
        event: VoiceEvent,
    ) -> Option<Self> {
        match (self, event) {
            (
                Self::Disconnected,
                VoiceEvent::ConnectRequested,
            ) => Some(Self::Connecting),

            (
                Self::Connecting,
                VoiceEvent::Connected,
            ) => Some(Self::Listening),

            (
                Self::Connecting,
                VoiceEvent::ConnectionFailed,
            ) => Some(Self::Disconnected),

            (_, VoiceEvent::Disconnected) => {
                Some(Self::Disconnected)
            }

            (
                Self::Listening,
                VoiceEvent::UserStartedTalking,
            ) => Some(Self::UserTalking),

            (
                Self::UserTalking,
                VoiceEvent::UserStoppedTalking,
            ) => Some(Self::Thinking),

            (
                Self::Thinking,
                VoiceEvent::ResponseStarted,
            ) => Some(Self::Speaking),

            (
                Self::Speaking,
                VoiceEvent::ResponseCompleted,
            ) => Some(Self::Listening),

            (
                Self::Thinking,
                VoiceEvent::ToolCallReceived,
            ) => Some(Self::Thinking),

            (
                Self::Thinking,
                VoiceEvent::ToolCallCompleted,
            ) => Some(Self::Thinking),

            (
                Self::Speaking,
                VoiceEvent::BargeInDetected,
            ) => Some(Self::BargeIn),

            (
                Self::BargeIn,
                VoiceEvent::UserStartedTalking,
            ) => Some(Self::UserTalking),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_lifecycle() {
        let state =
            VoiceState::Disconnected;

        let state = state
            .transition(
                VoiceEvent::ConnectRequested,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Connecting
        );

        let state = state
            .transition(
                VoiceEvent::Connected,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Listening
        );

        assert!(
            state.is_connected()
        );
    }

    #[test]
    fn connection_failure_returns_to_disconnected() {
        let state =
            VoiceState::Connecting;

        let state = state
            .transition(
                VoiceEvent::ConnectionFailed,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Disconnected
        );
    }

    #[test]
    fn normal_conversation_flow() {
        let state =
            VoiceState::Listening;

        let state = state
            .transition(
                VoiceEvent::UserStartedTalking,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::UserTalking
        );

        let state = state
            .transition(
                VoiceEvent::UserStoppedTalking,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Thinking
        );

        let state = state
            .transition(
                VoiceEvent::ResponseStarted,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Speaking
        );

        let state = state
            .transition(
                VoiceEvent::ResponseCompleted,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Listening
        );
    }

    #[test]
    fn tool_call_keeps_conversation_in_thinking() {
        let state =
            VoiceState::Thinking;

        let state = state
            .transition(
                VoiceEvent::ToolCallReceived,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Thinking
        );

        let state = state
            .transition(
                VoiceEvent::ToolCallCompleted,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::Thinking
        );
    }

    #[test]
    fn barge_in_interrupts_speaking() {
        let state =
            VoiceState::Speaking;

        let state = state
            .transition(
                VoiceEvent::BargeInDetected,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::BargeIn
        );

        let state = state
            .transition(
                VoiceEvent::UserStartedTalking,
            )
            .unwrap();

        assert_eq!(
            state,
            VoiceState::UserTalking
        );
    }

    #[test]
    fn disconnect_works_from_any_state() {
        let states = [
            VoiceState::Connecting,
            VoiceState::Listening,
            VoiceState::UserTalking,
            VoiceState::Thinking,
            VoiceState::Speaking,
            VoiceState::BargeIn,
        ];

        for state in states {
            assert_eq!(
                state.transition(
                    VoiceEvent::Disconnected,
                ),
                Some(
                    VoiceState::Disconnected
                )
            );
        }
    }

    #[test]
    fn invalid_transition_is_rejected() {
        assert_eq!(
            VoiceState::Listening
                .transition(
                    VoiceEvent::ResponseCompleted,
                ),
            None
        );

        assert_eq!(
            VoiceState::Speaking
                .transition(
                    VoiceEvent::UserStoppedTalking,
                ),
            None
        );

        assert_eq!(
            VoiceState::Disconnected
                .transition(
                    VoiceEvent::ResponseStarted,
                ),
            None
        );
    }

    #[test]
    fn busy_states_are_detected() {
        assert!(
            VoiceState::Connecting
                .is_busy()
        );

        assert!(
            VoiceState::UserTalking
                .is_busy()
        );

        assert!(
            VoiceState::Thinking
                .is_busy()
        );

        assert!(
            VoiceState::Speaking
                .is_busy()
        );

        assert!(
            VoiceState::BargeIn
                .is_busy()
        );

        assert!(
            !VoiceState::Disconnected
                .is_busy()
        );

        assert!(
            !VoiceState::Listening
                .is_busy()
        );
    }
}