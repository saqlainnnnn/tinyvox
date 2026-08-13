use super::cleanup::CleanedText;

pub trait TextInjector {
    type Error: std::error::Error + Send + Sync + 'static;

    fn prepare(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn inject(&self, text: &CleanedText) -> Result<(), Self::Error>;
}
