use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastDictation {
    text: String,
}

impl LastDictation {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn replace(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl Default for LastDictation {
    fn default() -> Self {
        Self::new("")
    }
}

pub type SharedLastDictation = Arc<RwLock<LastDictation>>;

pub fn shared() -> SharedLastDictation {
    Arc::new(RwLock::new(LastDictation::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_last_dictation() {
        let dictation = LastDictation::new("hello from TinyVox");

        assert_eq!(dictation.text(), "hello from TinyVox");
    }

    #[test]
    fn replaces_last_dictation() {
        let mut dictation = LastDictation::new("first dictation");

        dictation.replace("second dictation");

        assert_eq!(dictation.text(), "second dictation");
    }

    #[test]
    fn default_is_empty() {
        let dictation = LastDictation::default();

        assert!(dictation.is_empty());

        assert_eq!(dictation.text(), "");
    }

    #[test]
    fn shared_last_dictation_can_be_updated() {
        let shared = shared();

        {
            let mut dictation = shared.write().unwrap();

            dictation.replace("hello TinyVox");
        }

        let dictation = shared.read().unwrap();

        assert_eq!(dictation.text(), "hello TinyVox");
    }
}
