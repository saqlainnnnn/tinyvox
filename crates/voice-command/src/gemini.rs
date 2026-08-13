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
        build_websocket_url(&self.api_key)
    }
}

impl VoiceProvider for GeminiLiveProvider {
    type Error = GeminiError;
    type Session = GeminiLiveSession;

    fn connect(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::Session, Self::Error>> + Send {
        let url = self.websocket_url();

        let api_key = self.api_key.clone();

        let model = self.model.clone();

        async move {
            let url = url?;

            println!("🔌 Connecting to Gemini Live...");

            let (mut websocket, _response) = connect_async(&url).await?;

            println!("✓ Gemini WebSocket connected.");

            /*
             * Fresh connection:
             *
             * Empty sessionResumption tells Gemini that
             * this setup participates in resumption, but
             * there is no previous handle to resume.
             */
            let setup = build_setup(&model, None);

            println!("→ Gemini setup sent.");

            websocket
                .send(Message::Text(setup.to_string().into()))
                .await?;

            wait_for_setup_complete(&mut websocket).await?;

            println!("✓ Gemini Live session ready.");

            let (send_tx, send_rx) = mpsc::channel::<Message>(32);

            let (event_tx, event_rx) = mpsc::channel::<VoiceEvent>(128);

            tokio::spawn(run_gemini_transport(
                websocket, api_key, model, send_rx, event_tx,
            ));

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

        /*
         * Google documents realtime audio as:
         *
         * realtimeInput.audio
         *
         * with PCM at 16 kHz.
         */
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

    pub async fn start_activity(&self) -> Result<(), GeminiError> {
        let message = json!({
            "realtimeInput": {
                "activityStart": {}
            }
        });

        self.send_tx
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|_| GeminiError::ChannelClosed)
    }

    pub async fn end_activity(&self) -> Result<(), GeminiError> {
        let message = json!({
            "realtimeInput": {
                "activityEnd": {}
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
// Persistent Gemini transport
// =============================================================

async fn run_gemini_transport(
    mut websocket: GeminiWebSocket,

    api_key: String,

    model: String,

    mut send_rx: mpsc::Receiver<Message>,

    event_tx: mpsc::Sender<VoiceEvent>,
) {
    let mut resume_handle: Option<String> = None;

    loop {
        let result =
            run_connection(&mut websocket, &mut send_rx, &event_tx, &mut resume_handle).await;

        match result {
            Ok(()) => {
                println!("🛑 Gemini transport stopped.");

                break;
            }

            Err(error) => {
                eprintln!("⚠ Gemini connection lost: {error}");

                /*
                 * If the application closed the send side,
                 * don't reconnect.
                 */
                if send_rx.is_closed() {
                    break;
                }

                /*
                 * Reconnect only after an actual transport
                 * failure.
                 */
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                match reconnect_gemini(&api_key, &model, resume_handle.as_deref()).await {
                    Ok(new_websocket) => {
                        websocket = new_websocket;

                        println!("✓ Gemini session reconnected.");
                    }

                    Err(reconnect_error) => {
                        eprintln!("⚠ Gemini reconnect failed: {reconnect_error}");

                        /*
                         * Do NOT spin forever on a malformed
                         * session/setup. Surface the error to
                         * VoiceAgent and stop this transport.
                         */
                        let _ = event_tx
                            .send(VoiceEvent::Error(reconnect_error.to_string()))
                            .await;

                        break;
                    }
                }
            }
        }
    }
}

// =============================================================
// Reconnect
// =============================================================

async fn reconnect_gemini(
    api_key: &str,

    model: &str,

    resume_handle: Option<&str>,
) -> Result<GeminiWebSocket, GeminiError> {
    let url = build_websocket_url(api_key)?;

    println!("🔄 Connecting to Gemini again...");

    let (mut websocket, _response) = connect_async(&url).await?;

    let setup = build_setup(model, resume_handle);

    websocket
        .send(Message::Text(setup.to_string().into()))
        .await?;

    wait_for_setup_complete(&mut websocket).await?;

    Ok(websocket)
}

// =============================================================
// WebSocket loop
// =============================================================

async fn run_connection(
    websocket: &mut GeminiWebSocket,

    send_rx: &mut mpsc::Receiver<Message>,

    event_tx: &mpsc::Sender<VoiceEvent>,

    resume_handle: &mut Option<String>,
) -> Result<(), GeminiError> {
    loop {
        tokio::select! {
            outbound =
                send_rx.recv()
                => {
                match outbound {
                    Some(message) => {
                        websocket
                            .send(message)
                            .await?;
                    }

                    None => {
                        return Ok(());
                    }
                }
            }

            inbound =
                websocket.next()
                => {
                let message =
                    match inbound {
                        Some(
                            Ok(message),
                        ) => {
                            message
                        }

                        Some(
                            Err(error),
                        ) => {
                            return Err(
                                GeminiError::
                                    WebSocket(
                                        error,
                                    ),
                            );
                        }

                        None => {
                            return Err(
                                GeminiError::
                                    ConnectionClosed,
                            );
                        }
                    };

                match message {
                    Message::Text(
                        text,
                    ) => {
                        capture_transport_control(
                            &text,
                            resume_handle,
                        );

                        handle_server_message(
                            &text,
                            event_tx,
                        )
                        .await;
                    }

                    Message::Binary(
                        bytes,
                    ) => {
                        let text =
                            String::from_utf8_lossy(
                                &bytes,
                            );

                        capture_transport_control(
                            &text,
                            resume_handle,
                        );

                        handle_server_message(
                            &text,
                            event_tx,
                        )
                        .await;
                    }

                    Message::Ping(
                        payload,
                    ) => {
                        websocket
                            .send(
                                Message::Pong(
                                    payload,
                                ),
                            )
                            .await?;
                    }

                    Message::Pong(_) => {}

                    Message::Close(
                        frame,
                    ) => {
                        let reason =
                            frame
                                .map(
                                    |frame| {
                                        format!(
                                            "code={} reason={}",
                                            frame.code,
                                            frame.reason
                                        )
                                    },
                                )
                                .unwrap_or_else(
                                    || {
                                        "no close frame reason"
                                            .to_string()
                                    },
                                );

                        return Err(
                            GeminiError::
                                UnexpectedResponse(
                                    format!(
                                        "connection closed: {reason}"
                                    ),
                                ),
                        );
                    }

                    Message::Frame(_) => {}
                }
            }
        }
    }
}

// =============================================================
// Transport control
// =============================================================

fn capture_transport_control(text: &str, resume_handle: &mut Option<String>) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };

    /*
     * SessionResumptionUpdate is only sent when the
     * setup contained sessionResumption.
     *
     * Keep the newest usable handle.
     */
    if let Some(update) = value.get("sessionResumptionUpdate") {
        let resumable = update
            .get("resumable")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if resumable {
            if let Some(handle) = update.get("newHandle").and_then(Value::as_str) {
                *resume_handle = Some(handle.to_string());

                println!("🔐 Gemini resumption handle updated.");
            }
        }
    }

    /*
     * GoAway tells us the current WebSocket will soon end.
     *
     * We don't immediately tear down the connection here;
     * run_connection will naturally observe the close and
     * reconnect using the latest handle.
     */
    if let Some(go_away) = value.get("goAway") {
        if let Some(time_left) = go_away.get("timeLeft").and_then(Value::as_str) {
            println!("⚠ Gemini GoAway received; time left: {time_left}");
        } else {
            println!("⚠ Gemini GoAway received.");
        }
    }
}

// =============================================================
// Setup
// =============================================================

fn build_setup(model: &str, resume_handle: Option<&str>) -> Value {
    /*
     * Google documents:
     *
     *   sessionResumption: {}
     *
     * for a fresh resumable session, and:
     *
     *   sessionResumption: {
     *       handle: "..."
     *   }
     *
     * when resuming an existing session.
     */
    let session_resumption = match resume_handle {
        Some(handle) => {
            json!({
                "handle": handle
            })
        }

        None => {
            json!({})
        }
    };

    json!({
        "setup": {
            "model":
                format!(
                    "models/{model}"
                ),

            "generationConfig": {
                "responseModalities": [
                    "AUDIO"
                ]
            },

            /*
             * IMPORTANT:
             *
             * This is a union field. It must specify the
             * compression mechanism instead of being an empty
             * object.
             */
            "contextWindowCompression": {
                "slidingWindow": {}
            },

            /*
             * Manual client-side VAD.
             *
             * Google documents activityStart/activityEnd as
             * the required mechanism when automatic VAD is
             * disabled.
             */
            "realtimeInputConfig": {
                "automaticActivityDetection": {
                    "disabled": true
                }
            },

            "sessionResumption":
                session_resumption,

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
    })
}

fn build_websocket_url(api_key: &str) -> Result<String, GeminiError> {
    let url = format!(
        "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={api_key}"
    );

    url::Url::parse(&url).map_err(GeminiError::InvalidUrl)?;

    Ok(url)
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
        /*
         * Gemini sends interrupted=true when a client
         * activity message cuts off model generation.
         */
        if server_content.get("interrupted").and_then(Value::as_bool) == Some(true) {
            events.push(VoiceEvent::Interrupted);
        }

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

    /*
     * Function calling.
     */
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
    fn parses_interrupted() {
        let events = parse_server_message(
            r#"{
                    "serverContent": {
                        "interrupted": true
                    }
                }"#,
        )
        .unwrap();

        assert_eq!(events, vec![VoiceEvent::Interrupted]);
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

    #[test]
    fn fresh_setup_has_valid_compression_and_resumption() {
        let setup = build_setup("gemini-3.1-flash-live-preview", None);

        let setup = setup.get("setup").unwrap();

        assert!(
            setup
                .get("contextWindowCompression")
                .and_then(|value| value.get("slidingWindow"))
                .is_some()
        );

        assert_eq!(setup.get("sessionResumption").unwrap(), &json!({}));
    }

    #[test]
    fn resumed_setup_contains_handle() {
        let setup = build_setup("gemini-3.1-flash-live-preview", Some("resume-token"));

        assert_eq!(
            setup
                .get("setup")
                .unwrap()
                .get("sessionResumption")
                .unwrap(),
            &json!({
                "handle": "resume-token"
            })
        );
    }
}
