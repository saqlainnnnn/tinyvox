use std::env;

use futures_util::{SinkExt, StreamExt};

use serde_json::{Value, json};

use tokio::{net::TcpStream, sync::mpsc};

use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use crate::{
    VoiceState,
    ports::{AudioChunk, ToolCall, VoiceEvent, VoiceProvider, VoiceSession},
};

type GeminiWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug)]
pub enum GeminiError {
    MissingApiKey,

    WebSocket(tokio_tungstenite::tungstenite::Error),

    Json(serde_json::Error),

    ConnectionClosed,

    UnexpectedResponse(String),

    InvalidUrl(url::ParseError),

    ChannelClosed,
}

impl std::fmt::Display for GeminiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(f, "GEMINI_API_KEY is not set")
            }

            Self::WebSocket(error) => {
                write!(f, "Gemini WebSocket error: {error}")
            }

            Self::Json(error) => {
                write!(f, "Gemini JSON error: {error}")
            }

            Self::ConnectionClosed => {
                write!(f, "Gemini connection closed")
            }

            Self::UnexpectedResponse(response) => {
                write!(f, "unexpected Gemini response: {response}")
            }

            Self::InvalidUrl(error) => {
                write!(f, "invalid Gemini WebSocket URL: {error}")
            }

            Self::ChannelClosed => {
                write!(f, "Gemini session channel closed")
            }
        }
    }
}

impl std::error::Error for GeminiError {}

impl From<tokio_tungstenite::tungstenite::Error> for GeminiError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(error)
    }
}

impl From<serde_json::Error> for GeminiError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Clone)]
pub struct GeminiLiveProvider {
    api_key: String,
    model: String,
}

impl GeminiLiveProvider {
    pub fn from_env() -> Result<Self, GeminiError> {
        let api_key = env::var("GEMINI_API_KEY").map_err(|_| GeminiError::MissingApiKey)?;

        let model = env::var("GEMINI_LIVE_MODEL")
            .unwrap_or_else(|_| "gemini-3.1-flash-live-preview".to_string());

        Ok(Self { api_key, model })
    }

    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    fn websocket_url(&self) -> Result<String, GeminiError> {
        let url = format!(
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={}",
            self.api_key
        );

        url::Url::parse(&url).map_err(GeminiError::InvalidUrl)?;

        Ok(url)
    }
}

impl VoiceProvider for GeminiLiveProvider {
    type Error = GeminiError;
    type Session = GeminiLiveSession;

    fn connect(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::Session, Self::Error>> + Send {
        let url = self.websocket_url();

        let model = self.model.clone();

        async move {
            let url = url?;

            println!("🔌 Connecting to Gemini Live...");

            let (mut websocket, _response) = connect_async(&url).await?;

            println!("✓ Gemini WebSocket connected.");

            let setup = json!({
                "setup": {
                    "model": format!(
                        "models/{model}"
                    ),

                    "generationConfig": {
                        "responseModalities": [
                            "AUDIO"
                        ]
                    },

                    "tools": [
                        {
                            "functionDeclarations": [
                                {
                                    "name":
                                        "read_last_dictation",

                                    "description":
                                        "Returns the most recently injected dictation text.",

                                    "parameters": {
                                        "type":
                                            "OBJECT"
                                    }
                                },

                                {
                                    "name":
                                        "add_dictionary_entry",

                                    "description":
                                        "Adds a manual speech correction to the TinyVox dictionary.",

                                    "parameters": {
                                        "type":
                                            "OBJECT",

                                        "properties": {
                                            "wrong": {
                                                "type":
                                                    "STRING",
                                                "description":
                                                    "The phrase TinyVox commonly mishears."
                                            },

                                            "correct": {
                                                "type":
                                                    "STRING",
                                                "description":
                                                    "The correct replacement text."
                                            }
                                        },

                                        "required": [
                                            "wrong",
                                            "correct"
                                        ]
                                    }
                                }
                            ]
                        }
                    ]
                }
            });

            println!("→ Gemini setup sent.");

            websocket
                .send(Message::Text(setup.to_string().into()))
                .await?;

            wait_for_setup_complete(&mut websocket).await?;

            println!("✓ Gemini Live session ready.");

            let (websocket_writer, websocket_reader) = websocket.split();

            let (send_tx, mut send_rx) = mpsc::channel::<Message>(32);

            let (event_tx, event_rx) = mpsc::channel::<VoiceEvent>(128);

            // -------------------------------------------------
            // WebSocket writer
            // -------------------------------------------------

            tokio::spawn(async move {
                let mut writer = websocket_writer;

                while let Some(message) = send_rx.recv().await {
                    if writer.send(message).await.is_err() {
                        break;
                    }
                }
            });

            // -------------------------------------------------
            // WebSocket reader
            // -------------------------------------------------

            let pong_tx = send_tx.clone();

            tokio::spawn(async move {
                let mut reader = websocket_reader;

                while let Some(result) = reader.next().await {
                    let message = match result {
                        Ok(message) => message,

                        Err(error) => {
                            let _ = event_tx
                                .send(VoiceEvent::Error(format!(
                                    "Gemini WebSocket error: {error}"
                                )))
                                .await;

                            break;
                        }
                    };

                    match message {
                        Message::Text(text) => {
                            handle_server_message(&text, &event_tx).await;
                        }

                        Message::Binary(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);

                            handle_server_message(&text, &event_tx).await;
                        }

                        Message::Ping(payload) => {
                            let _ = pong_tx.send(Message::Pong(payload)).await;
                        }

                        Message::Close(frame) => {
                            let reason = frame
                                .map(|frame| format!("code={} reason={}", frame.code, frame.reason))
                                .unwrap_or_else(|| "no close frame reason".to_string());

                            let _ = event_tx
                                .send(VoiceEvent::Error(format!(
                                    "Gemini closed WebSocket: {reason}"
                                )))
                                .await;

                            break;
                        }

                        Message::Pong(_) => {}

                        Message::Frame(_) => {}
                    }
                }

                let _ = event_tx
                    .send(VoiceEvent::Error("Gemini receive task ended".to_string()))
                    .await;
            });

            Ok(GeminiLiveSession {
                send_tx,
                event_rx,
                state: VoiceState::Listening,
            })
        }
    }
}

// =============================================================
// Session
// =============================================================

pub struct GeminiLiveSession {
    send_tx: mpsc::Sender<Message>,

    event_rx: mpsc::Receiver<VoiceEvent>,

    state: VoiceState,
}

impl GeminiLiveSession {
    pub fn state(&self) -> VoiceState {
        self.state
    }

    pub fn split(self) -> (GeminiSendHandle, GeminiReceiveHandle) {
        (
            GeminiSendHandle {
                send_tx: self.send_tx,
            },
            GeminiReceiveHandle {
                event_rx: self.event_rx,
            },
        )
    }
}

// =============================================================
// Send handle
// =============================================================

#[derive(Clone)]
pub struct GeminiSendHandle {
    send_tx: mpsc::Sender<Message>,
}

impl GeminiSendHandle {
    pub async fn send_audio(&self, chunk: AudioChunk) -> Result<(), GeminiError> {
        let data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk.samples);

        let message = json!({
            "realtimeInput": {
                "audio": {
                    "data": data,
                    "mimeType":
                        "audio/pcm;rate=16000"
                }
            }
        });

        self.send_tx
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| GeminiError::ChannelClosed)
    }

    pub async fn send_text(&self, text: &str) -> Result<(), GeminiError> {
        let message = json!({
            "realtimeInput": {
                "text": text
            }
        });

        self.send_tx
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| GeminiError::ChannelClosed)
    }

    pub async fn end_audio(&self) -> Result<(), GeminiError> {
        let message = json!({
            "realtimeInput": {
                "audioStreamEnd": true
            }
        });

        self.send_tx
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| GeminiError::ChannelClosed)
    }

    pub async fn send_tool_response(
        &self,
        id: &str,
        name: &str,
        response: Value,
    ) -> Result<(), GeminiError> {
        let message = json!({
            "toolResponse": {
                "functionResponses": [
                    {
                        "id": id,
                        "name": name,
                        "response": response
                    }
                ]
            }
        });

        self.send_tx
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| GeminiError::ChannelClosed)
    }
}

// =============================================================
// Receive handle
// =============================================================

pub struct GeminiReceiveHandle {
    event_rx: mpsc::Receiver<VoiceEvent>,
}

impl GeminiReceiveHandle {
    pub async fn poll_event(&mut self) -> Result<VoiceEvent, GeminiError> {
        self.event_rx
            .recv()
            .await
            .ok_or(GeminiError::ConnectionClosed)
    }
}

// =============================================================
// VoiceSession compatibility
// =============================================================

impl VoiceSession for GeminiLiveSession {
    type Error = GeminiError;

    fn send_audio(
        &mut self,
        chunk: AudioChunk,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let sender = self.send_tx.clone();

        async move {
            let data =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &chunk.samples);

            let message = json!({
                "realtimeInput": {
                    "audio": {
                        "data": data,
                        "mimeType":
                            "audio/pcm;rate=16000"
                    }
                }
            });

            sender
                .send(Message::Text(message.to_string().into()))
                .await
                .map_err(|_| GeminiError::ChannelClosed)
        }
    }

    fn poll_event(
        &mut self,
    ) -> impl std::future::Future<Output = Result<VoiceEvent, Self::Error>> + Send {
        async move {
            self.event_rx
                .recv()
                .await
                .ok_or(GeminiError::ConnectionClosed)
        }
    }

    fn interrupt(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

// =============================================================
// Server messages
// =============================================================

async fn handle_server_message(text: &str, event_tx: &mpsc::Sender<VoiceEvent>) {
    match parse_server_message(text) {
        Ok(events) => {
            for event in events {
                if event_tx.send(event).await.is_err() {
                    return;
                }
            }
        }

        Err(error) => {
            let _ = event_tx.send(VoiceEvent::Error(error.to_string())).await;
        }
    }
}

async fn wait_for_setup_complete(websocket: &mut GeminiWebSocket) -> Result<(), GeminiError> {
    loop {
        let message = websocket
            .next()
            .await
            .ok_or(GeminiError::ConnectionClosed)??;

        match message {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(&text)?;

                if value.get("setupComplete").is_some() {
                    return Ok(());
                }

                if let Some(error) = value.get("error") {
                    return Err(GeminiError::UnexpectedResponse(error.to_string()));
                }
            }

            Message::Binary(bytes) => {
                let value: Value = serde_json::from_slice(&bytes)?;

                if value.get("setupComplete").is_some() {
                    return Ok(());
                }

                if let Some(error) = value.get("error") {
                    return Err(GeminiError::UnexpectedResponse(error.to_string()));
                }
            }

            Message::Ping(payload) => {
                websocket.send(Message::Pong(payload)).await?;
            }

            Message::Close(frame) => {
                let reason = frame
                    .map(|frame| format!("code={} reason={}", frame.code, frame.reason))
                    .unwrap_or_else(|| "no close frame reason".to_string());

                return Err(GeminiError::UnexpectedResponse(reason));
            }

            Message::Pong(_) => {}

            Message::Frame(_) => {}
        }
    }
}

fn parse_server_message(text: &str) -> Result<Vec<VoiceEvent>, GeminiError> {
    let value: Value = serde_json::from_str(text)?;

    let mut events = Vec::new();

    if let Some(error) = value.get("error") {
        events.push(VoiceEvent::Error(error.to_string()));

        return Ok(events);
    }

    if let Some(server_content) = value.get("serverContent") {
        if server_content.get("turnComplete").and_then(Value::as_bool) == Some(true) {
            events.push(VoiceEvent::TurnComplete);
        }

        if let Some(model_turn) = server_content.get("modelTurn") {
            if let Some(parts) = model_turn.get("parts").and_then(Value::as_array) {
                for part in parts {
                    let Some(inline_data) = part.get("inlineData") else {
                        continue;
                    };

                    let Some(data) = inline_data.get("data").and_then(Value::as_str) else {
                        continue;
                    };

                    let bytes =
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                            .map_err(|error| {
                                GeminiError::UnexpectedResponse(format!(
                                    "invalid audio base64: {error}"
                                ))
                            })?;

                    events.push(VoiceEvent::AudioOut(AudioChunk { samples: bytes }));
                }
            }
        }
    }

    if let Some(tool_call) = value.get("toolCall") {
        if let Some(function_calls) = tool_call.get("functionCalls").and_then(Value::as_array) {
            for function_call in function_calls {
                let id = function_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let name = function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let arguments = function_call
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| json!({}))
                    .to_string();

                events.push(VoiceEvent::ToolCall(ToolCall {
                    id,
                    name,
                    arguments,
                }));
            }
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_turn_complete() {
        let events = parse_server_message(
            r#"{
                    "serverContent": {
                        "turnComplete": true
                    }
                }"#,
        )
        .unwrap();

        assert_eq!(events, vec![VoiceEvent::TurnComplete]);
    }

    #[test]
    fn parses_audio_output() {
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8, 2u8, 3u8]);

        let message = format!(
            r#"{{
                    "serverContent": {{
                        "modelTurn": {{
                            "parts": [{{
                                "inlineData": {{
                                    "data": "{encoded}"
                                }}
                            }}]
                        }}
                    }}
                }}"#
        );

        let events = parse_server_message(&message).unwrap();

        assert_eq!(events.len(), 1);

        assert_eq!(
            events[0],
            VoiceEvent::AudioOut(AudioChunk {
                samples: vec![1, 2, 3],
            })
        );
    }

    #[test]
    fn parses_tool_call() {
        let events = parse_server_message(
            r#"{
                    "toolCall": {
                        "functionCalls": [
                            {
                                "id": "call-123",
                                "name": "read_last_dictation",
                                "args": {}
                            }
                        ]
                    }
                }"#,
        )
        .unwrap();

        assert_eq!(
            events,
            vec![VoiceEvent::ToolCall(ToolCall {
                id: "call-123".to_string(),
                name: "read_last_dictation".to_string(),
                arguments: "{}".to_string(),
            })]
        );
    }
}
