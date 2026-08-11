use std::env;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

const ELECTRON_URL: &str =
    "https://api.smallest.ai/waves/v1/chat/completions";

#[derive(Debug)]
pub enum ElectronError {
    MissingApiKey,
    Http(reqwest::Error),
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    InvalidResponse(reqwest::Error),
    EmptyResponse,
}

impl std::fmt::Display for ElectronError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(
                    f,
                    "SMALLEST_API_KEY environment variable is missing"
                )
            }

            Self::Http(error) => {
                write!(
                    f,
                    "Electron HTTP request failed: {error}"
                )
            }

            Self::Api { status, body } => {
                write!(
                    f,
                    "Electron API returned {status}: {body}"
                )
            }

            Self::InvalidResponse(error) => {
                write!(
                    f,
                    "failed to decode Electron response: {error}"
                )
            }

            Self::EmptyResponse => {
                write!(
                    f,
                    "Electron returned an empty response"
                )
            }
        }
    }
}

impl std::error::Error for ElectronError {}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: &'static str,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

pub struct ElectronCleaner {
    client: Client,
    api_key: String,
}

impl ElectronCleaner {
    pub fn from_env() -> Result<Self, ElectronError> {
        let api_key = env::var("SMALLEST_API_KEY")
            .map_err(|_| ElectronError::MissingApiKey)?;

        Ok(Self {
            client: Client::new(),
            api_key,
        })
    }

    async fn clean_transcript(
        &self,
        transcript: &Transcript,
    ) -> Result<CleanedText, ElectronError> {
        let request = ChatRequest {
            model: "electron",
            messages: vec![
                Message {
                    role: "system",
                    content: String::from(
                        "You clean speech-to-text transcripts for dictation. \
                         Preserve the speaker's meaning and wording. \
                         Fix punctuation, capitalization, spacing, and obvious \
                         transcription artifacts. Do not add information. \
                         Return only the cleaned text.",
                    ),
                },
                Message {
                    role: "user",
                    content: transcript.text.clone(),
                },
            ],
        };

        let response = self
            .client
            .post(ELECTRON_URL)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(ElectronError::Http)?;

        if !response.status().is_success() {
            let status = response.status();

            let body = response
                .text()
                .await
                .map_err(ElectronError::Http)?;

            return Err(ElectronError::Api {
                status,
                body,
            });
        }

        let result = response
            .json::<ChatResponse>()
            .await
            .map_err(ElectronError::InvalidResponse)?;

        let text = result
            .choices
            .first()
            .and_then(|choice| {
                choice.message.content.as_deref()
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or(ElectronError::EmptyResponse)?;

        Ok(CleanedText {
            text: text.to_string(),
        })
    }
}

impl TextCleaner for ElectronCleaner {
    type Error = ElectronError;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> impl std::future::Future<
        Output = Result<CleanedText, Self::Error>,
    > + Send {
        self.clean_transcript(transcript)
    }
}
