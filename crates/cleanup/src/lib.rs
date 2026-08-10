use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

pub struct BasicCleaner;

impl TextCleaner for BasicCleaner {
    type Error = std::convert::Infallible;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> Result<CleanedText, Self::Error> {
        Ok(CleanedText {
            text: transcript.text.trim().to_string(),
        })
    }
}