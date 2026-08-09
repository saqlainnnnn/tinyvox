use super::audio::AudioBuffer;

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
}

pub trait SpeechToText {
    type Error;

    fn transcribe(
        &self,
        audio: &AudioBuffer,
    ) -> impl std::future::Future<Output = Result<Transcript, Self::Error>> + Send;
}