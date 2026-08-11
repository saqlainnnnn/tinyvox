use reqwest::Client;
use serde::{Deserialize, Serialize};

use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

const DEFAULT_LLAMA_URL: &str =
    "http://127.0.0.1:8080/v1/chat/completions";

const DEFAULT_LLAMA_MODEL: &str = "local-cleaner";

#[derive(Debug)]
pub enum LocalLlamaError {
    Http(reqwest::Error),
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    InvalidResponse(reqwest::Error),
    EmptyResponse,
}

impl std::fmt::Display for LocalLlamaError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Http(error) => {
                write!(
                    f,
                    "local llama.cpp request failed: {error}"
                )
            }

            Self::Api { status, body } => {
                write!(
                    f,
                    "local llama.cpp returned {status}: {body}"
                )
            }

            Self::InvalidResponse(error) => {
                write!(
                    f,
                    "failed to decode llama.cpp response: {error}"
                )
            }

            Self::EmptyResponse => {
                write!(
                    f,
                    "local llama.cpp returned an empty response"
                )
            }
        }
    }
}

impl std::error::Error for LocalLlamaError {}

#[derive(Debug, Serialize)]
struct LlamaChatRequest {
    model: String,
    messages: Vec<LlamaMessage>,
}

#[derive(Debug, Serialize)]
struct LlamaMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LlamaChatResponse {
    choices: Vec<LlamaChoice>,
}

#[derive(Debug, Deserialize)]
struct LlamaChoice {
    message: LlamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct LlamaResponseMessage {
    content: Option<String>,
}

pub struct LocalLlamaCleaner {
    client: Client,
    url: String,
    model: String,
}

impl LocalLlamaCleaner {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            url: DEFAULT_LLAMA_URL.to_string(),
            model: DEFAULT_LLAMA_MODEL.to_string(),
        }
    }

    pub fn with_url(
        mut self,
        url: impl Into<String>,
    ) -> Self {
        self.url = url.into();
        self
    }

    pub fn with_model(
        mut self,
        model: impl Into<String>,
    ) -> Self {
        self.model = model.into();
        self
    }

    async fn clean_transcript(
        &self,
        transcript: &Transcript,
    ) -> Result<CleanedText, LocalLlamaError> {
        let request = LlamaChatRequest {
            model: self.model.clone(),
            messages: vec![
                LlamaMessage {
                    role: "system",
                    content: String::from(
                        "You clean speech-to-text transcripts for dictation. \
                         Preserve the speaker's exact meaning and wording. \
                         Fix punctuation, capitalization, spacing, and obvious \
                         speech-to-text artifacts. Do not add information. \
                         Do not answer the user's request. \
                         Return only the cleaned transcript.",
                    ),
                },
                LlamaMessage {
                    role: "user",
                    content: transcript.text.clone(),
                },
            ],
        };

        let response = self
            .client
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .map_err(LocalLlamaError::Http)?;

        if !response.status().is_success() {
            let status = response.status();

            let body = response
                .text()
                .await
                .map_err(LocalLlamaError::Http)?;

            return Err(LocalLlamaError::Api {
                status,
                body,
            });
        }

        let result = response
            .json::<LlamaChatResponse>()
            .await
            .map_err(LocalLlamaError::InvalidResponse)?;

        let text = result
            .choices
            .first()
            .and_then(|choice| {
                choice.message.content.as_deref()
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or(LocalLlamaError::EmptyResponse)?;

        Ok(CleanedText {
            text: text.to_string(),
        })
    }
}

impl Default for LocalLlamaCleaner {
    fn default() -> Self {
        Self::new()
    }
}

impl TextCleaner for LocalLlamaCleaner {
    type Error = LocalLlamaError;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> impl std::future::Future<
        Output = Result<CleanedText, Self::Error>,
    > + Send {
        self.clean_transcript(transcript)
    }
}