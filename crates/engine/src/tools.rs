#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRequest {
    ReadLastDictation,

    AddDictionaryEntry {
        wrong: String,
        correct: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResult {
    LastDictation {
        text: Option<String>,
    },

    DictionaryEntryAdded {
        wrong: String,
        correct: String,
    },

    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_last_dictation_request() {
        let request =
            ToolRequest::ReadLastDictation;

        assert_eq!(
            request,
            ToolRequest::ReadLastDictation
        );
    }

    #[test]
    fn add_dictionary_entry_request() {
        let request =
            ToolRequest::AddDictionaryEntry {
                wrong: "kubernets".to_string(),
                correct: "Kubernetes".to_string(),
            };

        assert_eq!(
            request,
            ToolRequest::AddDictionaryEntry {
                wrong: "kubernets".to_string(),
                correct: "Kubernetes".to_string(),
            }
        );
    }

    #[test]
    fn last_dictation_result() {
        let result =
            ToolResult::LastDictation {
                text: Some(
                    "hello from TinyVox"
                        .to_string(),
                ),
            };

        assert_eq!(
            result,
            ToolResult::LastDictation {
                text: Some(
                    "hello from TinyVox"
                        .to_string(),
                ),
            }
        );
    }

    #[test]
    fn dictionary_entry_added_result() {
        let result =
            ToolResult::DictionaryEntryAdded {
                wrong: "kubernets".to_string(),
                correct: "Kubernetes".to_string(),
            };

        assert_eq!(
            result,
            ToolResult::DictionaryEntryAdded {
                wrong: "kubernets".to_string(),
                correct: "Kubernetes".to_string(),
            }
        );
    }

    #[test]
    fn error_result() {
        let result =
            ToolResult::Error {
                message:
                    "dictionary entry already exists"
                        .to_string(),
            };

        assert_eq!(
            result,
            ToolResult::Error {
                message:
                    "dictionary entry already exists"
                        .to_string(),
            }
        );
    }
}