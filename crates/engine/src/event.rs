#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    RecordingStarted,
    RecordingStopped,

    TranscriptionStarted,
    TranscriptionCompleted,

    CleanupStarted,
    CleanupCompleted,

    InjectionStarted,
    InjectionCompleted,

    Failed,
}
