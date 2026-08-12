use std::env;

use futures_util::{
    SinkExt,
    StreamExt,
};

use serde_json::json;

use tokio::{
    net::TcpStream,
    sync::mpsc,
};

use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    MaybeTlsStream,
    WebSocketStream,
};

use crate::{
    ports::{
        AudioChunk,
        VoiceEvent,
        VoiceProvider,
        VoiceSession,
    },
    VoiceState,
};

type GeminiWebSocket =
    WebSocketStream<
        MaybeTlsStream<TcpStream>,
    >;

#[derive(Debug)]
pub enum GeminiError {
    MissingApiKey,

    WebSocket(
        tokio_tungstenite::tungstenite::Error,
    ),

    Json(serde_json::Error),

    ConnectionClosed,

    UnexpectedResponse(String),

    InvalidUrl(url::ParseError),

    ChannelClosed,
}

impl std::fmt::Display for GeminiError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(
                    f,
                    "GEMINI_API_KEY is not set"
                )
            }

            Self::WebSocket(error) => {
                write!(
                    f,
                    "Gemini WebSocket error: {error}"
                )
            }

            Self::Json(error) => {
                write!(
                    f,
                    "Gemini JSON error: {error}"
                )
            }

            Self::ConnectionClosed => {
                write!(
                    f,
                    "Gemini connection closed"
                )
            }

            Self::UnexpectedResponse(
                response,
            ) => {
                write!(
                    f,
                    "unexpected Gemini response: {response}"
                )
            }

            Self::InvalidUrl(error) => {
                write!(
                    f,
                    "invalid Gemini WebSocket URL: {error}"
                )
            }

            Self::ChannelClosed => {
                write!(
                    f,
                    "Gemini session channel closed"
                )
            }
        }
    }
}

impl std::error::Error
    for GeminiError
{
}

impl From<
    tokio_tungstenite::tungstenite::Error
> for GeminiError {
    fn from(
        error:
            tokio_tungstenite::tungstenite::Error,
    ) -> Self {
        Self::WebSocket(error)
    }
}

impl From<serde_json::Error>
    for GeminiError
{
    fn from(
        error: serde_json::Error,
    ) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Clone)]
pub struct GeminiLiveProvider {
    api_key: String,
    model: String,
}

impl GeminiLiveProvider {
    pub fn from_env()
        -> Result<Self, GeminiError>
    {
        let api_key =
            env::var("GEMINI_API_KEY")
                .map_err(|_| {
                    GeminiError::MissingApiKey
                })?;

        let model =
            env::var(
                "GEMINI_LIVE_MODEL",
            )
            .unwrap_or_else(|_| {
                "gemini-3.1-flash-live-preview"
                    .to_string()
            });

        Ok(Self {
            api_key,
            model,
        })
    }

    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    fn websocket_url(
        &self,
    ) -> Result<String, GeminiError> {
        let url =
            format!(
                "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={}",
                self.api_key
            );

        url::Url::parse(&url)
            .map_err(
                GeminiError::InvalidUrl,
            )?;

        Ok(url)
    }
}

impl VoiceProvider
    for GeminiLiveProvider
{
    type Error = GeminiError;
    type Session =
        GeminiLiveSession;

    fn connect(
        &self,
    ) -> impl std::future::Future<
        Output =
            Result<
                Self::Session,
                Self::Error,
            >,
    > + Send {
        let url =
            self.websocket_url();

        let model =
            self.model.clone();

        async move {
            let url = url?;

            println!(
                "🔌 Connecting to Gemini Live..."
            );

            let (
                mut websocket,
                _response,
            ) =
                connect_async(&url)
                    .await?;

            println!(
                "✓ Gemini WebSocket connected."
            );

            let setup =
                json!({
                    "setup": {
                        "model": format!(
                            "models/{model}"
                        ),
                        "generationConfig": {
                            "responseModalities": [
                                "AUDIO"
                            ]
                        }
                    }
                });

            println!(
                "→ Gemini setup sent."
            );

            websocket
                .send(
                    Message::Text(
                        setup
                            .to_string()
                            .into(),
                    ),
                )
                .await?;

            wait_for_setup_complete(
                &mut websocket,
            )
            .await?;

            println!(
                "✓ Gemini Live session ready."
            );

            let (
                websocket_writer,
                websocket_reader,
            ) = websocket.split();

            let (
                send_tx,
                mut send_rx,
            ) =
                mpsc::channel::<Message>(
                    32,
                );

            let (
                event_tx,
                event_rx,
            ) =
                mpsc::channel::<VoiceEvent>(
                    128,
                );

            // -------------------------------------------------
            // WebSocket writer task
            // -------------------------------------------------

            tokio::spawn(
                async move {
                    let mut writer =
                        websocket_writer;

                    while let Some(
                        message,
                    ) =
                        send_rx.recv().await
                    {
                        if writer
                            .send(message)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                },
            );

            // -------------------------------------------------
            // WebSocket reader task
            // -------------------------------------------------

            let pong_tx =
                send_tx.clone();

            tokio::spawn(
                async move {
                    let mut reader =
                        websocket_reader;

                    while let Some(
                        result,
                    ) =
                        reader.next().await
                    {
                        let message =
                            match result {
                                Ok(message) => {
                                    message
                                }

                                Err(error) => {
                                    let _ =
                                        event_tx
                                            .send(
                                                VoiceEvent::Error(
                                                    format!(
                                                        "Gemini WebSocket error: {error}"
                                                    ),
                                                ),
                                            )
                                            .await;

                                    break;
                                }
                            };

                        match message {
                            Message::Text(
                                text,
                            ) => {
                                match parse_server_message(
                                    &text,
                                ) {
                                    Ok(Some(
                                        event,
                                    )) => {
                                        if event_tx
                                            .send(event)
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }

                                    Ok(None) => {}

                                    Err(error) => {
                                        let _ =
                                            event_tx
                                                .send(
                                                    VoiceEvent::Error(
                                                        error.to_string(),
                                                    ),
                                                )
                                                .await;
                                    }
                                }
                            }

                            Message::Binary(
                                bytes,
                            ) => {
                                let text =
                                    String::from_utf8_lossy(
                                        &bytes,
                                    );

                                match parse_server_message(
                                    &text,
                                ) {
                                    Ok(Some(
                                        event,
                                    )) => {
                                        if event_tx
                                            .send(event)
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }

                                    Ok(None) => {}

                                    Err(error) => {
                                        let _ =
                                            event_tx
                                                .send(
                                                    VoiceEvent::Error(
                                                        error.to_string(),
                                                    ),
                                                )
                                                .await;
                                    }
                                }
                            }

                            Message::Ping(
                                payload,
                            ) => {
                                if pong_tx
                                    .send(
                                        Message::Pong(
                                            payload,
                                        ),
                                    )
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }

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

                                let _ =
                                    event_tx
                                        .send(
                                            VoiceEvent::Error(
                                                format!(
                                                    "Gemini closed WebSocket: {reason}"
                                                ),
                                            ),
                                        )
                                        .await;

                                break;
                            }

                            Message::Pong(_) => {}

                            Message::Frame(_) => {}
                        }
                    }

                    let _ =
                        event_tx
                            .send(
                                VoiceEvent::Error(
                                    "Gemini receive task ended"
                                        .to_string(),
                                ),
                            )
                            .await;
                },
            );

            Ok(
                GeminiLiveSession {
                    send_tx,
                    event_rx,
                    state:
                        VoiceState::Listening,
                },
            )
        }
    }
}

// =============================================================
// Full session
// =============================================================

pub struct GeminiLiveSession {
    send_tx:
        mpsc::Sender<Message>,

    event_rx:
        mpsc::Receiver<VoiceEvent>,

    state: VoiceState,
}

impl GeminiLiveSession {
    pub fn state(
        &self,
    ) -> VoiceState {
        self.state
    }

    pub fn split(
        self,
    ) -> (
        GeminiSendHandle,
        GeminiReceiveHandle,
    ) {
        (
            GeminiSendHandle {
                send_tx: self.send_tx,
            },
            GeminiReceiveHandle {
                event_rx:
                    self.event_rx,
            },
        )
    }
}

// =============================================================
// Send handle
// =============================================================

#[derive(Clone)]
pub struct GeminiSendHandle {
    send_tx:
        mpsc::Sender<Message>,
}

impl GeminiSendHandle {
    pub async fn send_audio(
        &self,
        chunk: AudioChunk,
    ) -> Result<(), GeminiError> {
        let data =
            base64::Engine::encode(
                &base64
                    ::engine
                    ::general_purpose
                    ::STANDARD,
                &chunk.samples,
            );

        let message =
            json!({
                "realtimeInput": {
                    "audio": {
                        "data": data,
                        "mimeType":
                            "audio/pcm;rate=16000"
                    }
                }
            });

        self.send_tx
            .send(
                Message::Text(
                    message
                        .to_string()
                        .into(),
                ),
            )
            .await
            .map_err(
                |_| {
                    GeminiError::
                        ChannelClosed
                },
            )?;

        Ok(())
    }
    pub async fn end_audio(
        &self,
    ) -> Result<(), GeminiError> {
        let message = json!({
            "realtimeInput": {
                "audioStreamEnd": true
            }
        });

        self.send_tx
            .send(Message::Text(
                message.to_string().into(),
            ))
            .await
            .map_err(|_| {
                GeminiError::ChannelClosed
            })?;

        Ok(())
    }


}

// =============================================================
// Receive handle
// =============================================================

pub struct GeminiReceiveHandle {
    event_rx:
        mpsc::Receiver<VoiceEvent>,
}

impl GeminiReceiveHandle {
    pub async fn poll_event(
        &mut self,
    ) -> Result<
        VoiceEvent,
        GeminiError,
    > {
        self.event_rx
            .recv()
            .await
            .ok_or(
                GeminiError::
                    ConnectionClosed,
            )
    }
}

// =============================================================
// Existing VoiceSession implementation
// =============================================================

impl VoiceSession
    for GeminiLiveSession
{
    type Error = GeminiError;

    fn send_audio(
        &mut self,
        chunk: AudioChunk,
    ) -> impl std::future::Future<
        Output =
            Result<(), Self::Error>,
    > + Send {
        let sender =
            self.send_tx.clone();

        async move {
            let data =
                base64::Engine::encode(
                    &base64
                        ::engine
                        ::general_purpose
                        ::STANDARD,
                    &chunk.samples,
                );

            let message =
                json!({
                    "realtimeInput": {
                        "audio": {
                            "data": data,
                            "mimeType":
                                "audio/pcm;rate=16000"
                        }
                    }
                });

            sender
                .send(
                    Message::Text(
                        message
                            .to_string()
                            .into(),
                    ),
                )
                .await
                .map_err(
                    |_| {
                        GeminiError::
                            ChannelClosed
                    },
                )
        }
    }

    fn poll_event(
        &mut self,
    ) -> impl std::future::Future<
        Output =
            Result<
                VoiceEvent,
                Self::Error,
            >,
    > + Send {
        async move {
            self.event_rx
                .recv()
                .await
                .ok_or(
                    GeminiError::
                        ConnectionClosed,
                )
        }
    }

    fn interrupt(
        &mut self,
    ) -> impl std::future::Future<
        Output =
            Result<(), Self::Error>,
    > + Send {
        async move {
            Ok(())
        }
    }
}

// =============================================================
// Setup
// =============================================================

async fn wait_for_setup_complete(
    websocket: &mut GeminiWebSocket,
) -> Result<(), GeminiError> {
    loop {
        let message =
            websocket
                .next()
                .await
                .ok_or(
                    GeminiError::
                        ConnectionClosed,
                )??;

        match message {
            Message::Text(text) => {
                let value:
                    serde_json::Value =
                    serde_json::from_str(
                        &text,
                    )?;

                if value
                    .get("setupComplete")
                    .is_some()
                {
                    return Ok(());
                }

                if let Some(error) =
                    value.get("error")
                {
                    return Err(
                        GeminiError::
                            UnexpectedResponse(
                                error.to_string(),
                            ),
                    );
                }
            }

            Message::Binary(bytes) => {
                let value:
                    serde_json::Value =
                    serde_json::from_slice(
                        &bytes,
                    )?;

                if value
                    .get("setupComplete")
                    .is_some()
                {
                    return Ok(());
                }

                if let Some(error) =
                    value.get("error")
                {
                    return Err(
                        GeminiError::
                            UnexpectedResponse(
                                error.to_string(),
                            ),
                    );
                }
            }

            Message::Ping(payload) => {
                websocket
                    .send(
                        Message::Pong(
                            payload,
                        ),
                    )
                    .await?;
            }

            Message::Close(frame) => {
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
                                "Gemini closed WebSocket during setup: {reason}"
                            ),
                        ),
                );
            }

            Message::Pong(_) => {}

            Message::Frame(_) => {}
        }
    }
}

// =============================================================
// Server event parser
// =============================================================

fn parse_server_message(
    text: &str,
) -> Result<
    Option<VoiceEvent>,
    GeminiError,
> {
    let value:
        serde_json::Value =
        serde_json::from_str(text)?;

    if let Some(error) =
        value.get("error")
    {
        return Ok(Some(
            VoiceEvent::Error(
                error.to_string(),
            ),
        ));
    }

    if let Some(server_content) =
        value.get("serverContent")
    {
        if server_content
            .get("turnComplete")
            .and_then(
                serde_json::Value::as_bool,
            )
            == Some(true)
        {
            return Ok(Some(
                VoiceEvent::TurnComplete,
            ));
        }

        if let Some(model_turn) =
            server_content
                .get("modelTurn")
        {
            if let Some(parts) =
                model_turn
                    .get("parts")
                    .and_then(
                        serde_json::Value::as_array,
                    )
            {
                for part in parts {
                    if let Some(
                        inline_data,
                    ) =
                        part.get(
                            "inlineData",
                        )
                    {
                        if let Some(data) =
                            inline_data
                                .get("data")
                                .and_then(
                                    serde_json::Value::as_str,
                                )
                        {
                            let bytes =
                                base64::Engine::decode(
                                    &base64
                                        ::engine
                                        ::general_purpose
                                        ::STANDARD,
                                    data,
                                )
                                .map_err(
                                    |error| {
                                        GeminiError::
                                            UnexpectedResponse(
                                                format!(
                                                    "invalid audio base64: {error}"
                                                ),
                                            )
                                    },
                                )?;

                            return Ok(Some(
                                VoiceEvent::AudioOut(
                                    AudioChunk {
                                        samples:
                                            bytes,
                                    },
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_turn_complete() {
        let event =
            parse_server_message(
                r#"{
                    "serverContent": {
                        "turnComplete": true
                    }
                }"#,
            )
            .unwrap();

        assert_eq!(
            event,
            Some(
                VoiceEvent::TurnComplete
            )
        );
    }

    #[test]
    fn parses_audio_output() {
        let encoded =
            base64::Engine::encode(
                &base64
                    ::engine
                    ::general_purpose
                    ::STANDARD,
                [1u8, 2u8, 3u8],
            );

        let message =
            format!(
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

        let event =
            parse_server_message(
                &message,
            )
            .unwrap();

        assert_eq!(
            event,
            Some(
                VoiceEvent::AudioOut(
                    AudioChunk {
                        samples:
                            vec![1, 2, 3],
                    },
                ),
            )
        );
    }
}