pub mod gemini;
pub mod ports;

pub use gemini::{
    GeminiError, GeminiLiveProvider, GeminiLiveSession, GeminiReceiveHandle, GeminiSendHandle,
};

pub use ports::{AudioChunk, ToolCall, VoiceEvent, VoiceProvider, VoiceSession};

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
pub enum VoiceStateEvent {
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
        !matches!(self, Self::Disconnected)
    }

    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::UserTalking | Self::Thinking | Self::Speaking | Self::BargeIn
        )
    }

    pub fn transition(self, event: VoiceStateEvent) -> Option<Self> {
        match (self, event) {
            (Self::Disconnected, VoiceStateEvent::ConnectRequested) => Some(Self::Connecting),

            (Self::Connecting, VoiceStateEvent::Connected) => Some(Self::Listening),

            (Self::Connecting, VoiceStateEvent::ConnectionFailed) => Some(Self::Disconnected),

            (_, VoiceStateEvent::Disconnected) => Some(Self::Disconnected),

            (Self::Listening, VoiceStateEvent::UserStartedTalking) => Some(Self::UserTalking),

            (Self::UserTalking, VoiceStateEvent::UserStoppedTalking) => Some(Self::Thinking),

            (Self::Thinking, VoiceStateEvent::ResponseStarted) => Some(Self::Speaking),

            (Self::Speaking, VoiceStateEvent::ResponseCompleted) => Some(Self::Listening),

            (Self::Thinking, VoiceStateEvent::ToolCallReceived) => Some(Self::Thinking),

            (Self::Thinking, VoiceStateEvent::ToolCallCompleted) => Some(Self::Thinking),

            (Self::Speaking, VoiceStateEvent::BargeInDetected) => Some(Self::BargeIn),

            (Self::BargeIn, VoiceStateEvent::UserStartedTalking) => Some(Self::UserTalking),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_lifecycle() {
        let state = VoiceState::Disconnected;

        let state = state.transition(VoiceStateEvent::ConnectRequested).unwrap();

        assert_eq!(state, VoiceState::Connecting);

        let state = state.transition(VoiceStateEvent::Connected).unwrap();

        assert_eq!(state, VoiceState::Listening);

        assert!(state.is_connected());
    }

    #[test]
    fn connection_failure_returns_to_disconnected() {
        let state = VoiceState::Connecting;

        let state = state.transition(VoiceStateEvent::ConnectionFailed).unwrap();

        assert_eq!(state, VoiceState::Disconnected);
    }

    #[test]
    fn normal_conversation_flow() {
        let state = VoiceState::Listening;

        let state = state
            .transition(VoiceStateEvent::UserStartedTalking)
            .unwrap();

        assert_eq!(state, VoiceState::UserTalking);

        let state = state
            .transition(VoiceStateEvent::UserStoppedTalking)
            .unwrap();

        assert_eq!(state, VoiceState::Thinking);

        let state = state.transition(VoiceStateEvent::ResponseStarted).unwrap();

        assert_eq!(state, VoiceState::Speaking);

        let state = state
            .transition(VoiceStateEvent::ResponseCompleted)
            .unwrap();

        assert_eq!(state, VoiceState::Listening);
    }

    #[test]
    fn tool_call_keeps_conversation_in_thinking() {
        let state = VoiceState::Thinking;

        let state = state.transition(VoiceStateEvent::ToolCallReceived).unwrap();

        assert_eq!(state, VoiceState::Thinking);

        let state = state
            .transition(VoiceStateEvent::ToolCallCompleted)
            .unwrap();

        assert_eq!(state, VoiceState::Thinking);
    }

    #[test]
    fn barge_in_interrupts_speaking() {
        let state = VoiceState::Speaking;

        let state = state.transition(VoiceStateEvent::BargeInDetected).unwrap();

        assert_eq!(state, VoiceState::BargeIn);

        let state = state
            .transition(VoiceStateEvent::UserStartedTalking)
            .unwrap();

        assert_eq!(state, VoiceState::UserTalking);
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
                state.transition(VoiceStateEvent::Disconnected,),
                Some(VoiceState::Disconnected)
            );
        }
    }

    #[test]
    fn invalid_transition_is_rejected() {
        assert_eq!(
            VoiceState::Listening.transition(VoiceStateEvent::ResponseCompleted,),
            None
        );

        assert_eq!(
            VoiceState::Speaking.transition(VoiceStateEvent::UserStoppedTalking,),
            None
        );

        assert_eq!(
            VoiceState::Disconnected.transition(VoiceStateEvent::ResponseStarted,),
            None
        );
    }

    #[test]
    fn busy_states_are_detected() {
        assert!(VoiceState::Connecting.is_busy());

        assert!(VoiceState::UserTalking.is_busy());

        assert!(VoiceState::Thinking.is_busy());

        assert!(VoiceState::Speaking.is_busy());

        assert!(VoiceState::BargeIn.is_busy());

        assert!(!VoiceState::Disconnected.is_busy());

        assert!(!VoiceState::Listening.is_busy());
    }
}
