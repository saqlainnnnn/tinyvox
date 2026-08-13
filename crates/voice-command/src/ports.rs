use crate::VoiceState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub samples: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceEvent {
    AudioOut(AudioChunk),
    ToolCall(ToolCall),
    TurnComplete,
    Interrupted,
    StateChanged(VoiceState),
    Error(String),
}

pub trait VoiceProvider {
    type Error;
    type Session: VoiceSession<Error = Self::Error>;

    fn connect(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::Session, Self::Error>> + Send;
}

pub trait VoiceSession {
    type Error;

    fn send_audio(
        &mut self,
        chunk: AudioChunk,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    fn poll_event(
        &mut self,
    ) -> impl std::future::Future<Output = Result<VoiceEvent, Self::Error>> + Send;

    fn interrupt(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}
