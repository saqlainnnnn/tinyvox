use std::env;

use reqwest::Client;
use serde::{Deserialize, Serialize};

use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

/* ============================================================
 * Electron
 * ============================================================
 */

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
struct ElectronChatRequest {
    model: &'static str,
    messages: Vec<ElectronMessage>,
}

#[derive(Debug, Serialize)]
struct ElectronMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ElectronChatResponse {
    choices: Vec<ElectronChoice>,
}

#[derive(Debug, Deserialize)]
struct ElectronChoice {
    message: ElectronResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ElectronResponseMessage {
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
        let request = ElectronChatRequest {
            model: "electron",
            messages: vec![
                ElectronMessage {
                    role: "system",
                    content: String::from(
                        "You clean speech-to-text transcripts for dictation. \
                         Preserve the speaker's meaning and wording. \
                         Fix punctuation, capitalization, spacing, and obvious \
                         transcription artifacts. Do not add information. \
                         Return only the cleaned text.",
                    ),
                },
                ElectronMessage {
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
            .json::<ElectronChatResponse>()
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

/* ============================================================
 * Groq
 * ============================================================
 */

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

/* ============================================================
 * Basic deterministic cleaner
 * ============================================================
 */

pub struct BasicCleaner;

impl TextCleaner for BasicCleaner {
    type Error = std::convert::Infallible;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> impl std::future::Future<
        Output = Result<CleanedText, Self::Error>,
    > + Send {
        async move {
            Ok(CleanedText {
                text: transcript.text.trim().to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    EmptyOutput,
    OutputTooLong,
    ExcessiveExpansion,
}

impl std::fmt::Display for ValidationError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::EmptyOutput => {
                write!(f, "cleaned text is empty")
            }

            Self::OutputTooLong => {
                write!(f, "cleaned text is too long")
            }

            Self::ExcessiveExpansion => {
                write!(f, "cleaned text expanded excessively")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_cleaned_text(
    transcript: &Transcript,
    cleaned: &CleanedText,
) -> Result<(), ValidationError> {
    let original = transcript.text.trim();
    let output = cleaned.text.trim();

    // Never inject an empty response.
    if output.is_empty() {
        return Err(ValidationError::EmptyOutput);
    }

    // Hard upper bound.
    if output.len() > 10_000 {
        return Err(ValidationError::OutputTooLong);
    }

    // The cleaner should not suddenly turn a short dictation
    // into a massive block of generated text.
    //
    // Allow normal expansion for punctuation/formatting,
    // but reject extreme expansion.
    if !original.is_empty()
        && output.len() > original.len() * 3
    {
        return Err(
            ValidationError::ExcessiveExpansion,
        );
    }

    Ok(())
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn accepts_normal_cleanup() {
        let transcript = Transcript {
            text: "hello how are you".to_string(),
        };

        let cleaned = CleanedText {
            text: "Hello, how are you?".to_string(),
        };

        assert!(
            validate_cleaned_text(
                &transcript,
                &cleaned
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_empty_output() {
        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let cleaned = CleanedText {
            text: "   ".to_string(),
        };

        assert_eq!(
            validate_cleaned_text(
                &transcript,
                &cleaned
            ),
            Err(ValidationError::EmptyOutput)
        );
    }

    #[test]
    fn rejects_excessive_expansion() {
        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let cleaned = CleanedText {
            text: "hello ".repeat(20),
        };

        assert_eq!(
            validate_cleaned_text(
                &transcript,
                &cleaned
            ),
            Err(ValidationError::ExcessiveExpansion)
        );
    }

    #[test]
    fn accepts_reasonable_expansion() {
        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let cleaned = CleanedText {
            text: "Hello!".to_string(),
        };

        assert!(
            validate_cleaned_text(
                &transcript,
                &cleaned
            )
            .is_ok()
        );
    }
}