use super::cleanup::CleanedText;

pub trait TextInjector {
    type Error;

    fn inject(&self, text: &CleanedText) -> Result<(), Self::Error>;
}