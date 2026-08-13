use crate::{
    dictionary::{EntrySource, SharedDictionary},
    last_dictation::SharedLastDictation,
    tools::{ToolRequest, ToolResult},
};

#[derive(Debug, Clone)]
pub struct ToolRegistry {
    dictionary: SharedDictionary,
    last_dictation: SharedLastDictation,
}

impl ToolRegistry {
    pub fn new(dictionary: SharedDictionary, last_dictation: SharedLastDictation) -> Self {
        Self {
            dictionary,
            last_dictation,
        }
    }

    pub fn execute(&mut self, request: ToolRequest) -> ToolResult {
        match request {
            ToolRequest::ReadLastDictation => {
                let last_dictation = self.last_dictation.read().unwrap();

                ToolResult::LastDictation {
                    text: if last_dictation.is_empty() {
                        None
                    } else {
                        Some(last_dictation.text().to_string())
                    },
                }
            }

            ToolRequest::AddDictionaryEntry { wrong, correct } => {
                if wrong.trim().is_empty() {
                    return ToolResult::Error {
                        message: "dictionary entry cannot have an empty wrong value".to_string(),
                    };
                }

                if correct.trim().is_empty() {
                    return ToolResult::Error {
                        message: "dictionary entry cannot have an empty correct value".to_string(),
                    };
                }

                {
                    let mut dictionary = self.dictionary.write().unwrap();

                    dictionary.add(&wrong, &correct, EntrySource::Manual);
                }

                ToolResult::DictionaryEntryAdded { wrong, correct }
            }
        }
    }

    pub fn dictionary(&self) -> SharedDictionary {
        self.dictionary.clone()
    }

    pub fn last_dictation(&self) -> SharedLastDictation {
        self.last_dictation.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dictionary::shared, last_dictation::shared as shared_last_dictation};

    #[test]
    fn reads_empty_last_dictation() {
        let dictionary = shared();

        let last_dictation = shared_last_dictation();

        let mut registry = ToolRegistry::new(dictionary, last_dictation);

        let result = registry.execute(ToolRequest::ReadLastDictation);

        assert_eq!(result, ToolResult::LastDictation { text: None });
    }

    #[test]
    fn reads_last_dictation() {
        let dictionary = shared();

        let last_dictation = shared_last_dictation();

        {
            let mut last = last_dictation.write().unwrap();

            last.replace("hello from TinyVox");
        }

        let mut registry = ToolRegistry::new(dictionary, last_dictation);

        let result = registry.execute(ToolRequest::ReadLastDictation);

        assert_eq!(
            result,
            ToolResult::LastDictation {
                text: Some("hello from TinyVox".to_string(),),
            }
        );
    }

    #[test]
    fn adds_dictionary_entry() {
        let dictionary = shared();

        let last_dictation = shared_last_dictation();

        let mut registry = ToolRegistry::new(dictionary.clone(), last_dictation);

        let result = registry.execute(ToolRequest::AddDictionaryEntry {
            wrong: "kubernets".to_string(),
            correct: "Kubernetes".to_string(),
        });

        assert_eq!(
            result,
            ToolResult::DictionaryEntryAdded {
                wrong: "kubernets".to_string(),
                correct: "Kubernetes".to_string(),
            }
        );

        let mut dictionary = dictionary.write().unwrap();

        assert_eq!(dictionary.entries().len(), 1);

        assert_eq!(dictionary.apply("I use Kubernets."), "I use Kubernetes.");
    }

    #[test]
    fn rejects_empty_wrong_value() {
        let mut registry = ToolRegistry::new(shared(), shared_last_dictation());

        let result = registry.execute(ToolRequest::AddDictionaryEntry {
            wrong: "   ".to_string(),
            correct: "Kubernetes".to_string(),
        });

        assert_eq!(
            result,
            ToolResult::Error {
                message: "dictionary entry cannot have an empty wrong value".to_string(),
            }
        );
    }

    #[test]
    fn rejects_empty_correct_value() {
        let mut registry = ToolRegistry::new(shared(), shared_last_dictation());

        let result = registry.execute(ToolRequest::AddDictionaryEntry {
            wrong: "kubernets".to_string(),
            correct: "   ".to_string(),
        });

        assert_eq!(
            result,
            ToolResult::Error {
                message: "dictionary entry cannot have an empty correct value".to_string(),
            }
        );
    }
}
