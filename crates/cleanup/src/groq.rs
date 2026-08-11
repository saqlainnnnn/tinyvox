use std::env;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

const GROQ_URL: &str =
    "https://api.groq.com/openai/v1/chat/completions";

const GROQ_MODEL: &str = "openai/gpt-oss-20b";

#[derive(Debug)]
pub enum GroqError {
    MissingApiKey,
    Http(reqwest::Error),
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    InvalidResponse(reqwest::Error),
    EmptyResponse,
}

impl std::fmt::Display for GroqError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(
                    f,
                    "GROQ_API_KEY environment variable is missing"
                )
            }

            Self::Http(error) => {
                write!(
                    f,
                    "Groq HTTP request failed: {error}"
                )
            }

            Self::Api { status, body } => {
                write!(
                    f,
                    "Groq API returned {status}: {body}"
                )
            }

            Self::InvalidResponse(error) => {
                write!(
                    f,
                    "failed to decode Groq response: {error}"
                )
            }

            Self::EmptyResponse => {
                write!(
                    f,
                    "Groq returned an empty response"
                )
            }
        }
    }
}

impl std::error::Error for GroqError {}

#[derive(Debug, Serialize)]
struct GroqChatRequest {
    model: &'static str,
    messages: Vec<GroqMessage>,
}

#[derive(Debug, Serialize)]
struct GroqMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct GroqChatResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqResponseMessage,
}

#[derive(Debug, Deserialize)]
struct GroqResponseMessage {
    content: Option<String>,
}

pub struct GroqCleaner {
    client: Client,
    api_key: String,
}

impl GroqCleaner {
    pub fn from_env() -> Result<Self, GroqError> {
        let api_key = env::var("GROQ_API_KEY")
            .map_err(|_| GroqError::MissingApiKey)?;

        Ok(Self {
            client: Client::new(),
            api_key,
        })
    }

    async fn clean_transcript(
        &self,
        transcript: &Transcript,
    ) -> Result<CleanedText, GroqError> {
        let request = GroqChatRequest {
            model: GROQ_MODEL,
            messages: vec![
                GroqMessage {
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
                GroqMessage {
                    role: "user",
                    content: transcript.text.clone(),
                },
            ],
        };

        let response = self
            .client
            .post(GROQ_URL)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(GroqError::Http)?;

        if !response.status().is_success() {
            let status = response.status();

            let body = response
                .text()
                .await
                .map_err(GroqError::Http)?;

            return Err(GroqError::Api {
                status,
                body,
            });
        }

        let result = response
            .json::<GroqChatResponse>()
            .await
            .map_err(GroqError::InvalidResponse)?;

        let text = result
            .choices
            .first()
            .and_then(|choice| {
                choice.message.content.as_deref()
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or(GroqError::EmptyResponse)?;

        Ok(CleanedText {
            text: text.to_string(),
        })
    }
}

impl TextCleaner for GroqCleaner {
    type Error = GroqError;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> impl std::future::Future<
        Output = Result<CleanedText, Self::Error>,
    > + Send {
        self.clean_transcript(transcript)
    }
}
