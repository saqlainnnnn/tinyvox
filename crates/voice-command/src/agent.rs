use serde_json::{Value, json};

use tokio::{
    sync::{mpsc, oneshot},
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

pub struct VoiceAgent {
    provider: GeminiLiveProvider,
    tool_registry: Option<ToolRegistry>,
    session: Option<AgentSession>,
}

struct AgentSession {
    stop_tx: Option<oneshot::Sender<()>>,

    sender_task: JoinHandle<Result<(), VoiceAgentError>>,

    receiver_task: JoinHandle<Result<(), VoiceAgentError>>,
}

impl VoiceAgent {
    pub fn new(provider: GeminiLiveProvider, tool_registry: ToolRegistry) -> Self {
        Self {
            provider,
            tool_registry: Some(tool_registry),
            session: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub async fn start(&mut self) -> Result<(), VoiceAgentError> {
        if self.session.is_some() {
            return Ok(());
        }

        let session = self.provider.connect().await?;

        let (send_handle, receive_handle) = session.split();

        let (stop_tx, stop_rx) = oneshot::channel::<()>();

        let (speech_tx, speech_rx) = mpsc::channel::<()>(8);

        let (receiver_stop_tx, receiver_stop_rx) = oneshot::channel::<()>();

        let tool_registry = self
            .tool_registry
            .take()
            .ok_or_else(|| VoiceAgentError::Task("tool registry unavailable".to_string()))?;

        let receiver_send_handle = send_handle.clone();

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

        println!("🛑 Voice agent stopped.");

        Ok(())
    }
}

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
                if speaking {
                    send_handle
                        .end_activity()
                        .await?;
                }

                microphone.stop();

                let _ =
                    receiver_stop_tx
                        .send(());

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
                    pcm16_rms(&chunk);

                let speech =
                    rms >= SPEECH_THRESHOLD;

                if speech {
                    silence_ms = 0;

                    if !speaking {
                        speaking = true;

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
                                samples: chunk,
                            },
                        )
                        .await?;
                } else if speaking {
                    send_handle
                        .send_audio(
                            AudioChunk {
                                samples: chunk,
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
            }
        }
    }

    Ok(())
}

async fn receiver_loop(
    send_handle: GeminiSendHandle,
    mut receive_handle: GeminiReceiveHandle,
    mut tool_registry: ToolRegistry,
    mut speech_rx: mpsc::Receiver<()>,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<(), VoiceAgentError> {
    let mut playback =
        CpalAudioPlayback::new().map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    playback
        .start()
        .map_err(|error| VoiceAgentError::Audio(error.to_string()))?;

    println!("🔊 Speaker playback ready.");

    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                playback.stop();

                println!(
                    "🔊 Speaker playback stopped."
                );

                break;
            }

            Some(_) =
                speech_rx.recv()
            => {
                playback.stop();

                playback
                    .start()
                    .map_err(|error| {
                        VoiceAgentError::Audio(
                            error.to_string(),
                        )
                    })?;

                println!(
                    "⚡ Barge-in: local VAD cleared playback."
                );
            }

            event =
                receive_handle.poll_event()
            => {
                let event = event?;

                match event {
                    VoiceEvent::AudioOut(
                        chunk,
                    ) => {
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
                        playback.stop();

                        playback
                            .start()
                            .map_err(|error| {
                                VoiceAgentError::Audio(
                                    error.to_string(),
                                )
                            })?;

                        println!(
                            "⚡ Barge-in: Gemini interrupted."
                        );
                    }

                    VoiceEvent::ToolCall(
                        tool_call,
                    ) => {
                        execute_tool_call(
                            &send_handle,
                            &mut tool_registry,
                            &tool_call,
                        )
                        .await?;
                    }

                    VoiceEvent::TurnComplete => {
                        println!(
                            "✓ Voice turn complete."
                        );
                    }

                    VoiceEvent::Error(
                        error,
                    ) => {
                        playback.stop();

                        return Err(
                            VoiceAgentError::Gemini(
                                GeminiError::UnexpectedResponse(
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
