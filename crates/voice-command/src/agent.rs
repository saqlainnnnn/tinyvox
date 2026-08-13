use serde_json::{Value, json};

use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Duration, sleep},
};

use audio::{CpalAudioPlayback, CpalAudioStreamer};

use tinyvox_engine::{tool_registry::ToolRegistry, tools::ToolRequest};

use crate::{
    AudioChunk, GeminiError, GeminiLiveProvider, GeminiReceiveHandle, GeminiSendHandle, VoiceEvent,
    VoiceProvider,
};

const CHUNK_INTERVAL_MS: u64 = 40;
const SPEECH_THRESHOLD: f32 = 0.015;
const SILENCE_TO_END_MS: u64 = 500;

// =============================================================
// Voice agent state
// =============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceAgentState {
    Listening,
    Thinking,
    Speaking,
    BargeIn,
    Stopped,
}

// =============================================================
// Errors
// =============================================================

#[derive(Debug)]
pub enum VoiceAgentError {
    Gemini(GeminiError),
    Audio(String),
    Tool(String),
    Task(String),
}

impl std::fmt::Display for VoiceAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gemini(error) => {
                write!(f, "Gemini error: {error}")
            }

            Self::Audio(error) => {
                write!(f, "audio error: {error}")
            }

            Self::Tool(error) => {
                write!(f, "tool error: {error}")
            }

            Self::Task(error) => {
                write!(f, "voice agent task error: {error}")
            }
        }
    }
}

impl std::error::Error for VoiceAgentError {}

impl From<GeminiError> for VoiceAgentError {
    fn from(error: GeminiError) -> Self {
        Self::Gemini(error)
    }
}

// =============================================================
// Voice agent
// =============================================================

pub struct VoiceAgent {
    provider: GeminiLiveProvider,

    tool_registry: Option<ToolRegistry>,

    session: Option<AgentSession>,

    state_tx: watch::Sender<VoiceAgentState>,
}

struct AgentSession {
    stop_tx: Option<oneshot::Sender<()>>,

    sender_task: JoinHandle<Result<(), VoiceAgentError>>,

    receiver_task: JoinHandle<Result<(), VoiceAgentError>>,
}

impl VoiceAgent {
    pub fn new(provider: GeminiLiveProvider, tool_registry: ToolRegistry) -> Self {
        let (state_tx, _state_rx) = watch::channel(VoiceAgentState::Stopped);

        Self {
            provider,
            tool_registry: Some(tool_registry),
            session: None,
            state_tx,
        }
    }

    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub fn state(&self) -> VoiceAgentState {
        *self.state_tx.borrow()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<VoiceAgentState> {
        self.state_tx.subscribe()
    }

    fn set_state(&self, state: VoiceAgentState) {
        let _ = self.state_tx.send(state);
    }

    pub async fn start(&mut self) -> Result<(), VoiceAgentError> {
        if self.session.is_some() {
            return Ok(());
        }

        let session = self.provider.connect().await?;

        let (send_handle, receive_handle) = session.split();

        let (stop_tx, stop_rx) = oneshot::channel::<()>();

        /*
         * Local VAD -> receiver notification.
         *
         * When user speech is detected while Gemini is speaking,
         * the receiver clears/restarts playback immediately.
         */
        let (speech_tx, speech_rx) = mpsc::channel::<()>(8);

        /*
         * Microphone task tells receiver task when the whole
         * agent is shutting down.
         */
        let (receiver_stop_tx, receiver_stop_rx) = oneshot::channel::<()>();

        let tool_registry = self
            .tool_registry
            .take()
            .ok_or_else(|| VoiceAgentError::Task("tool registry unavailable".to_string()))?;

        let receiver_send_handle = send_handle.clone();

        let state_tx = self.state_tx.clone();

        self.set_state(VoiceAgentState::Listening);

        let sender_task = tokio::spawn(microphone_loop(
            send_handle,
            stop_rx,
            speech_tx,
            receiver_stop_tx,
        ));

        let receiver_task = tokio::spawn(receiver_loop(
            receiver_send_handle,
            receive_handle,
            tool_registry,
            speech_rx,
            receiver_stop_rx,
            state_tx,
        ));

        self.session = Some(AgentSession {
            stop_tx: Some(stop_tx),

            sender_task,

            receiver_task,
        });

        println!("🎙️ Voice agent started.");

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), VoiceAgentError> {
        let Some(mut session) = self.session.take() else {
            self.set_state(VoiceAgentState::Stopped);

            return Ok(());
        };

        if let Some(stop_tx) = session.stop_tx.take() {
            let _ = stop_tx.send(());
        }

        session
            .sender_task
            .await
            .map_err(|error| VoiceAgentError::Task(error.to_string()))??;

        session
            .receiver_task
            .await
            .map_err(|error| VoiceAgentError::Task(error.to_string()))??;

        self.set_state(VoiceAgentState::Stopped);

        println!("🛑 Voice agent stopped.");

        Ok(())
    }
}

// =============================================================
// Microphone
// =============================================================

async fn microphone_loop(
    send_handle: GeminiSendHandle,

    mut stop_rx: oneshot::Receiver<()>,

    speech_tx: mpsc::Sender<()>,

    receiver_stop_tx: oneshot::Sender<()>,
) -> Result<(), VoiceAgentError> {
    let mut microphone =
        CpalAudioStreamer::new().map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    microphone
        .start()
        .map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    println!("🎙️ Microphone streaming...");

    let mut speaking = false;

    let mut silence_ms = 0u64;

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                /*
                 * In manual-VAD mode, close the current
                 * activity rather than sending audioStreamEnd.
                 */
                if speaking {
                    send_handle
                        .end_activity()
                        .await?;
                }

                microphone.stop();

                let _ =
                    receiver_stop_tx.send(());

                break;
            }

            _ = sleep(
                Duration::from_millis(
                    CHUNK_INTERVAL_MS,
                ),
            ) => {
                let chunk =
                    microphone
                        .read_chunk()
                        .map_err(|error| {
                            VoiceAgentError::Audio(
                                error.to_string(),
                            )
                        })?;

                if chunk.is_empty() {
                    continue;
                }

                let rms =
                    pcm16_rms(
                        &chunk,
                    );

                let speech =
                    rms >= SPEECH_THRESHOLD;

                if speech {
                    silence_ms = 0;

                    if !speaking {
                        speaking = true;

                        /*
                         * Notify receiver BEFORE sending
                         * activityStart so queued Gemini
                         * speech is cleared as early as possible.
                         */
                        let _ =
                            speech_tx
                                .send(())
                                .await;

                        send_handle
                            .start_activity()
                            .await?;

                        println!(
                            "⚡ User speech detected."
                        );
                    }

                    send_handle
                        .send_audio(
                            AudioChunk {
                                samples:
                                    chunk,
                            },
                        )
                        .await?;
                } else if speaking {
                    /*
                     * Send the tail of the activity before ending it.
                     */
                    send_handle
                        .send_audio(
                            AudioChunk {
                                samples:
                                    chunk,
                            },
                        )
                        .await?;

                    silence_ms +=
                        CHUNK_INTERVAL_MS;

                    if silence_ms
                        >= SILENCE_TO_END_MS
                    {
                        speaking = false;
                        silence_ms = 0;

                        send_handle
                            .end_activity()
                            .await?;

                        println!(
                            "→ User activity ended."
                        );
                    }
                }

                /*
                 * When speaking == false, intentionally send
                 * no realtime audio until the next activityStart.
                 */
            }
        }
    }

    Ok(())
}

// =============================================================
// Receiver / playback
// =============================================================

async fn receiver_loop(
    send_handle: GeminiSendHandle,

    mut receive_handle: GeminiReceiveHandle,

    mut tool_registry: ToolRegistry,

    mut speech_rx: mpsc::Receiver<()>,

    mut stop_rx: oneshot::Receiver<()>,

    state_tx: watch::Sender<VoiceAgentState>,
) -> Result<(), VoiceAgentError> {
    let mut playback =
        CpalAudioPlayback::new().map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    playback
        .start()
        .map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    println!("🔊 Speaker playback ready.");

    let _ = state_tx.send(VoiceAgentState::Listening);

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                playback.stop();

                let _ =
                    state_tx.send(
                        VoiceAgentState::Stopped,
                    );

                println!(
                    "🔊 Speaker playback stopped."
                );

                break;
            }

            Some(_) =
                speech_rx.recv()
            => {
                /*
                 * Local VAD says the user has started speaking.
                 *
                 * Immediately cut current Gemini output.
                 */
                let _ =
                    state_tx.send(
                        VoiceAgentState::BargeIn,
                    );

                playback.stop();

                playback
                    .start()
                    .map_err(|error| {
                        VoiceAgentError::Audio(
                            error.to_string(),
                        )
                    })?;

                /*
                 * Go immediately back to Listening; BargeIn is
                 * useful as a transient UI state but the microphone
                 * is now waiting for the user's turn.
                 */
                let _ =
                    state_tx.send(
                        VoiceAgentState::Listening,
                    );

                println!(
                    "⚡ Barge-in: local VAD cleared playback."
                );
            }

            event =
                receive_handle.poll_event()
            => {
                let event =
                    event?;

                match event {
                    VoiceEvent::AudioOut(
                        chunk,
                    ) => {
                        /*
                         * The model is actively producing speech.
                         */
                        let _ =
                            state_tx.send(
                                VoiceAgentState::Speaking,
                            );

                        playback
                            .push_pcm16(
                                &chunk.samples,
                            )
                            .map_err(|error| {
                                VoiceAgentError::Audio(
                                    error.to_string(),
                                )
                            })?;
                    }

                    VoiceEvent::Interrupted => {
                        let _ =
                            state_tx.send(
                                VoiceAgentState::BargeIn,
                            );

                        playback.stop();

                        playback
                            .start()
                            .map_err(|error| {
                                VoiceAgentError::Audio(
                                    error.to_string(),
                                )
                            })?;

                        let _ =
                            state_tx.send(
                                VoiceAgentState::Listening,
                            );

                        println!(
                            "⚡ Barge-in: Gemini interrupted."
                        );
                    }

                    VoiceEvent::ToolCall(
                        tool_call,
                    ) => {
                        /*
                         * Tool execution is model-side reasoning.
                         */
                        let _ =
                            state_tx.send(
                                VoiceAgentState::Thinking,
                            );

                        execute_tool_call(
                            &send_handle,
                            &mut tool_registry,
                            &tool_call,
                        )
                        .await?;
                    }

                    VoiceEvent::TurnComplete => {
                        /*
                         * The model completed its response.
                         * Microphone remains active and ready for
                         * the next user activity.
                         */
                        let _ =
                            state_tx.send(
                                VoiceAgentState::Listening,
                            );

                        println!(
                            "✓ Voice turn complete."
                        );
                    }

                    VoiceEvent::Error(
                        error,
                    ) => {
                        playback.stop();

                        let _ =
                            state_tx.send(
                                VoiceAgentState::Stopped,
                            );

                        return Err(
                            VoiceAgentError::Gemini(
                                GeminiError::
                                    UnexpectedResponse(
                                        error,
                                    ),
                            ),
                        );
                    }

                    _ => {}
                }
            }
        }
    }

    Ok(())
}

// =============================================================
// PCM RMS
// =============================================================

fn pcm16_rms(pcm: &[u8]) -> f32 {
    let mut sum = 0.0f64;

    let mut count = 0usize;

    for sample in pcm.chunks_exact(2) {
        let value = i16::from_le_bytes([sample[0], sample[1]]) as f64 / i16::MAX as f64;

        sum += value * value;

        count += 1;
    }

    if count == 0 {
        return 0.0;
    }

    (sum / count as f64).sqrt() as f32
}

// =============================================================
// Tools
// =============================================================

async fn execute_tool_call(
    send_handle: &GeminiSendHandle,

    registry: &mut ToolRegistry,

    tool_call: &crate::ports::ToolCall,
) -> Result<(), VoiceAgentError> {
    let arguments: Value = serde_json::from_str(&tool_call.arguments)
        .map_err(|error| VoiceAgentError::Tool(format!("invalid tool arguments: {error}")))?;

    let request = match tool_call.name.as_str() {
        "read_last_dictation" => ToolRequest::ReadLastDictation,

        "add_dictionary_entry" => {
            let wrong = arguments
                .get("wrong")
                .and_then(Value::as_str)
                .ok_or_else(|| VoiceAgentError::Tool("missing 'wrong' argument".to_string()))?;

            let correct = arguments
                .get("correct")
                .and_then(Value::as_str)
                .ok_or_else(|| VoiceAgentError::Tool("missing 'correct' argument".to_string()))?;

            ToolRequest::AddDictionaryEntry {
                wrong: wrong.to_string(),
                correct: correct.to_string(),
            }
        }

        unknown => {
            send_handle
                .send_tool_response(
                    &tool_call.id,
                    &tool_call.name,
                    json!({
                        "error": format!(
                            "unknown tool: {unknown}"
                        )
                    }),
                )
                .await?;

            return Ok(());
        }
    };

    println!("🔧 Tool call: {}", tool_call.name);

    let result = registry.execute(request);

    println!("   Tool result: {:?}", result);

    let response = serde_json::to_value(&result).map_err(|error| {
        VoiceAgentError::Tool(format!("failed to serialize tool result: {error}"))
    })?;

    send_handle
        .send_tool_response(
            &tool_call.id,
            &tool_call.name,
            json!({
                "result": response
            }),
        )
        .await?;

    println!("→ Tool response sent.");

    Ok(())
}
