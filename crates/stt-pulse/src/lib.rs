use std::env;

use reqwest::{Client, Url};
use serde::Deserialize;

use tinyvox_engine::ports::{AudioBuffer, SpeechToText, Transcript};

const PULSE_URL: &str = "https://api.smallest.ai/waves/v1/stt/";

#[derive(Debug)]
pub enum PulseError {
    MissingApiKey,
    InvalidUrl(url::ParseError),
    Http(reqwest::Error),
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    InvalidResponse(reqwest::Error),
}

impl std::fmt::Display for PulseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                write!(f, "SMALLEST_API_KEY environment variable is missing")
            }

            Self::InvalidUrl(error) => {
                write!(f, "failed to construct Pulse API URL: {error}")
            }

            Self::Http(error) => {
                write!(f, "Pulse HTTP request failed: {error}")
            }

            Self::Api { status, body } => {
                write!(f, "Pulse API returned {status}: {body}")
            }

            Self::InvalidResponse(error) => {
                write!(f, "failed to decode Pulse response: {error}")
            }
        }
    }
}

impl std::error::Error for PulseError {}

#[derive(Debug, Deserialize)]
struct PulseResponse {
    transcription: String,
}

pub struct PulseClient {
    client: Client,
    api_key: String,
    language: String,
}

impl PulseClient {
    pub fn from_env() -> Result<Self, PulseError> {
        let api_key = env::var("SMALLEST_API_KEY").map_err(|_| PulseError::MissingApiKey)?;

        Ok(Self {
            client: Client::new(),
            api_key,
            language: "en".to_string(),
        })
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    async fn transcribe_audio(&self, audio: &AudioBuffer) -> Result<Transcript, PulseError> {
        const MAX_ATTEMPTS: usize = 3;

        let body = wav_bytes(audio);

        let mut url = Url::parse(PULSE_URL).map_err(PulseError::InvalidUrl)?;

        url.query_pairs_mut()
            .append_pair("model", "pulse")
            .append_pair("language", &self.language);

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .client
                .post(url.clone())
                .bearer_auth(&self.api_key)
                .header("Content-Type", "application/octet-stream")
                .body(body.clone())
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        let result = response
                            .json::<PulseResponse>()
                            .await
                            .map_err(PulseError::InvalidResponse)?;

                        return Ok(Transcript {
                            text: result.transcription,
                        });
                    }

                    if !is_retryable_status(status) || attempt + 1 == MAX_ATTEMPTS {
                        let body = response.text().await.map_err(PulseError::Http)?;

                        return Err(PulseError::Api { status, body });
                    }
                }

                Err(error) => {
                    if attempt + 1 == MAX_ATTEMPTS {
                        return Err(PulseError::Http(error));
                    }
                }
            }

            let backoff_ms = 100 * (1 << attempt);

            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        }

        unreachable!()
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

impl SpeechToText for PulseClient {
    type Error = PulseError;

    fn transcribe(
        &self,
        audio: &AudioBuffer,
    ) -> impl std::future::Future<Output = Result<Transcript, Self::Error>> + Send {
        self.transcribe_audio(audio)
    }
}

fn wav_bytes(audio: &AudioBuffer) -> Vec<u8> {
    let pcm = pcm16_bytes(audio);

    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;

    let bytes_per_sample = bits_per_sample / 8;

    let block_align = channels * bytes_per_sample;

    let byte_rate = audio.sample_rate * channels as u32 * bytes_per_sample as u32;

    let data_size = pcm.len() as u32;

    let riff_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + pcm.len());

    // RIFF
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&audio.sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());

    // data
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(&pcm);

    wav
}

fn pcm16_bytes(audio: &AudioBuffer) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(audio.samples.len() * 2);

    for &sample in &audio.samples {
        let sample = sample.clamp(-1.0, 1.0);

        let pcm = (sample * i16::MAX as f32) as i16;

        bytes.extend_from_slice(&pcm.to_le_bytes());
    }

    bytes
}
