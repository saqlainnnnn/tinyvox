use tinyvox_engine::ports::{
    CleanedText,
    Transcript,
};

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

    if output.is_empty() {
        return Err(ValidationError::EmptyOutput);
    }

    if output.len() > 10_000 {
        return Err(ValidationError::OutputTooLong);
    }

    if !original.is_empty()
        && output.len() > original.len() * 3
    {
        return Err(ValidationError::ExcessiveExpansion);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
