use std::env;

use futures_util::{
    SinkExt,
    StreamExt,
};
use serde_json::json;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    WebSocketStream,
};
use tokio_tungstenite::MaybeTlsStream;
use tokio::net::TcpStream;

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

            Self::UnexpectedResponse(response) => {
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
        }
    }
}

impl std::error::Error for GeminiError {}

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
            .map_err(GeminiError::InvalidUrl)?;

        Ok(url)
    }
}

impl VoiceProvider for GeminiLiveProvider {
    type Error = GeminiError;
    type Session = GeminiLiveSession;

    fn connect(
        &self,
    ) -> impl std::future::Future<
        Output =
            Result<
                Self::Session,
                Self::Error,
            >,
    > + Send {
        let url = self
            .websocket_url();

        let model =
            self.model.clone();

        async move {
            let url = url?;

            println!(
                "🔌 Connecting to Gemini Live..."
            );

            let (mut websocket, _) =
                connect_async(&url)
                    .await?;

            println!(
                "✓ Gemini WebSocket connected."
            );

            let setup = json!({
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

            websocket
                .send(Message::Text(
                    setup.to_string().into(),
                ))
                .await?;

            println!(
                "→ Gemini setup sent."
            );

            wait_for_setup_complete(
                &mut websocket,
            )
            .await?;

            println!(
                "✓ Gemini Live session ready."
            );

            Ok(
                GeminiLiveSession {
                    websocket,
                    state:
                        VoiceState::Listening,
                },
            )
        }
    }
}

pub struct GeminiLiveSession {
    websocket: GeminiWebSocket,
    state: VoiceState,
}

impl GeminiLiveSession {
    pub fn state(
        &self,
    ) -> VoiceState {
        self.state
    }

    async fn send_json(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), GeminiError> {
        self.websocket
            .send(Message::Text(
                value.to_string().into(),
            ))
            .await?;

        Ok(())
    }
}

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
        let data =
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &chunk.samples,
            );

        async move {
            self.send_json(
                json!({
                    "realtimeInput": {
                        "audio": {
                            "data": data,
                            "mimeType":
                                "audio/pcm;rate=16000"
                        }
                    }
                }),
            )
            .await
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
            loop {
                let message =
                    self.websocket
                        .next()
                        .await
                        .ok_or(
                            GeminiError::ConnectionClosed
                        )??;

                match message {
                    Message::Text(text) => {
                        if let Some(event) =
                            parse_server_message(
                                &text,
                            )?
                        {
                            return Ok(event);
                        }
                    }

                    Message::Binary(bytes) => {
                        if let Some(event) =
                            parse_server_message(
                                &String::from_utf8_lossy(
                                    &bytes,
                                ),
                            )?
                        {
                            return Ok(event);
                        }
                    }

                    Message::Ping(payload) => {
                        self.websocket
                            .send(
                                Message::Pong(
                                    payload,
                                ),
                            )
                            .await?;
                    }

                    Message::Close(_) => {
                        self.state =
                            VoiceState::Disconnected;

                        return Err(
                            GeminiError::ConnectionClosed
                        );
                    }

                    Message::Pong(_) => {}

                    Message::Frame(_) => {}
                }
            }
        }
    }

    fn interrupt(
        &mut self,
    ) -> impl std::future::Future<
        Output =
            Result<(), Self::Error>,
    > + Send {
        async move {
            /*
             * Gemini Live currently handles turn
             * interruption through realtime input /
             * activity handling. We are intentionally
             * keeping the provider boundary here so
             * barge-in implementation can be added
             * without changing the state machine.
             */
            Ok(())
        }
    }
}

async fn wait_for_setup_complete(
    websocket: &mut GeminiWebSocket,
) -> Result<(), GeminiError> {
    loop {
        let message =
            websocket
                .next()
                .await
                .ok_or(
                    GeminiError::ConnectionClosed,
                )??;

        match message {
            Message::Text(text) => {
                println!(
                    "← Gemini: {}",
                    text
                );

                let value:
                    serde_json::Value =
                    serde_json::from_str(&text)?;

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
                        GeminiError::UnexpectedResponse(
                            error.to_string(),
                        ),
                    );
                }
            }

            Message::Binary(bytes) => {
                let text =
                    String::from_utf8_lossy(
                        &bytes,
                    );

                println!(
                    "← Gemini binary: {}",
                    text
                );

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
                        GeminiError::UnexpectedResponse(
                            error.to_string(),
                        ),
                    );
                }
            }

            Message::Ping(payload) => {
                websocket
                    .send(
                        Message::Pong(payload),
                    )
                    .await?;
            }

            Message::Close(frame) => {
                let reason = frame
                    .map(|frame| {
                        format!(
                            "code={} reason={}",
                            frame.code,
                            frame.reason
                        )
                    })
                    .unwrap_or_else(|| {
                        "no close frame reason"
                            .to_string()
                    });

                return Err(
                    GeminiError::UnexpectedResponse(
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

fn parse_server_message(
    text: &str,
) -> Result<
    Option<VoiceEvent>,
    GeminiError,
> {
    let value:
        serde_json::Value =
        serde_json::from_str(text)?;

    if value
        .get("error")
        .is_some()
    {
        return Ok(Some(
            VoiceEvent::Error(
                value["error"].to_string(),
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
                    if let Some(inline_data) =
                        part.get("inlineData")
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
                                    &base64::engine::general_purpose::STANDARD,
                                    data,
                                )
                                .map_err(|error| {
                                    GeminiError::UnexpectedResponse(
                                        format!(
                                            "invalid audio base64: {error}"
                                        ),
                                    )
                                })?;

                            return Ok(Some(
                                VoiceEvent::AudioOut(
                                    AudioChunk {
                                        samples: bytes,
                                    },
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    if value
        .get("toolCall")
        .is_some()
    {
        return Ok(None);
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
                &base64::engine::general_purpose::STANDARD,
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

    #[test]
    fn provider_reads_environment() {
        let provider =
            GeminiLiveProvider::new(
                "test-key",
                "test-model",
            );

        let url =
            provider
                .websocket_url()
                .unwrap();

        assert!(
            url.contains(
                "test-key"
            )
        );

        assert!(
            url.contains(
                "GenerativeService.BidiGenerateContent"
            )
        );
    }
}