use super::Transcript;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanedText {
    pub text: String,
}

pub trait TextCleaner {
    type Error;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> Result<CleanedText, Self::Error>;
}