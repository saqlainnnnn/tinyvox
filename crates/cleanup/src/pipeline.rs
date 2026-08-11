use std::convert::Infallible;

use tinyvox_engine::ports::{
    CleanedText,
    TextCleaner,
    Transcript,
};

use crate::validation::validate_cleaned_text;

#[derive(Debug)]
pub enum CleanupError<PE, FE> {
    Primary(PE),
    Fallback(FE),
    ValidationFailed,
}

impl<PE: std::fmt::Display, FE: std::fmt::Display>
    std::fmt::Display for CleanupError<PE, FE>
{
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Primary(error) => {
                write!(
                    f,
                    "primary cleanup failed: {error}"
                )
            }

            Self::Fallback(error) => {
                write!(
                    f,
                    "fallback cleanup failed: {error}"
                )
            }

            Self::ValidationFailed => {
                write!(
                    f,
                    "both cleanup outputs failed validation"
                )
            }
        }
    }
}

impl<PE, FE> std::error::Error
    for CleanupError<PE, FE>
where
    PE: std::error::Error + 'static,
    FE: std::error::Error + 'static,
{
}

pub struct CleanupPipeline<P, F>
where
    P: TextCleaner,
    F: TextCleaner,
{
    primary: P,
    fallback: F,
}

impl<P, F> CleanupPipeline<P, F>
where
    P: TextCleaner,
    F: TextCleaner,
{
    pub fn new(
        primary: P,
        fallback: F,
    ) -> Self {
        Self {
            primary,
            fallback,
        }
    }

    pub async fn clean(
        &self,
        transcript: &Transcript,
    ) -> CleanedText {
        if let Ok(cleaned) =
            self.primary.clean(transcript).await
        {
            if validate_cleaned_text(
                transcript,
                &cleaned,
            )
            .is_ok()
            {
                return cleaned;
            }
        }

        if let Ok(cleaned) =
            self.fallback.clean(transcript).await
        {
            if validate_cleaned_text(
                transcript,
                &cleaned,
            )
            .is_ok()
            {
                return cleaned;
            }
        }

        CleanedText {
            text: transcript.text.trim().to_string(),
        }
    }
}

impl<P, F> TextCleaner for CleanupPipeline<P, F>
where
    P: TextCleaner + Sync,
    F: TextCleaner + Sync,
{
    type Error = Infallible;

    fn clean(
        &self,
        transcript: &Transcript,
    ) -> impl std::future::Future<
        Output = Result<CleanedText, Self::Error>,
    > + Send {
        async move {
            Ok(self.clean(transcript).await)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SuccessfulCleaner;

    impl TextCleaner for SuccessfulCleaner {
        type Error = &'static str;

        async fn clean(
            &self,
            transcript: &Transcript,
        ) -> Result<CleanedText, Self::Error> {
            Ok(CleanedText {
                text: format!(
                    "Cleaned: {}",
                    transcript.text
                ),
            })
        }
    }

    struct FailingCleaner;

    impl TextCleaner for FailingCleaner {
        type Error = &'static str;

        async fn clean(
            &self,
            _transcript: &Transcript,
        ) -> Result<CleanedText, Self::Error> {
            Err("cleanup failed")
        }
    }

    struct InvalidCleaner;

    impl TextCleaner for InvalidCleaner {
        type Error = &'static str;

        async fn clean(
            &self,
            _transcript: &Transcript,
        ) -> Result<CleanedText, Self::Error> {
            Ok(CleanedText {
                text: String::new(),
            })
        }
    }

    #[tokio::test]
    async fn primary_success_is_used() {
        let pipeline = CleanupPipeline::new(
            SuccessfulCleaner,
            FailingCleaner,
        );

        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let result =
            pipeline.clean(&transcript).await;

        assert_eq!(
            result.text,
            "Cleaned: hello"
        );
    }

    #[tokio::test]
    async fn failed_primary_uses_fallback() {
        let pipeline = CleanupPipeline::new(
            FailingCleaner,
            SuccessfulCleaner,
        );

        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let result =
            pipeline.clean(&transcript).await;

        assert_eq!(
            result.text,
            "Cleaned: hello"
        );
    }

    #[tokio::test]
    async fn invalid_primary_uses_fallback() {
        let pipeline = CleanupPipeline::new(
            InvalidCleaner,
            SuccessfulCleaner,
        );

        let transcript = Transcript {
            text: "hello".to_string(),
        };

        let result =
            pipeline.clean(&transcript).await;

        assert_eq!(
            result.text,
            "Cleaned: hello"
        );
    }

    #[tokio::test]
    async fn both_fail_returns_raw_transcript() {
        let pipeline = CleanupPipeline::new(
            FailingCleaner,
            FailingCleaner,
        );

        let transcript = Transcript {
            text: "  hello from TinyVox  "
                .to_string(),
        };

        let result =
            pipeline.clean(&transcript).await;

        assert_eq!(
            result.text,
            "hello from TinyVox"
        );
    }

    #[tokio::test]
    async fn invalid_primary_and_fallback_returns_raw_transcript() {
        let pipeline = CleanupPipeline::new(
            InvalidCleaner,
            InvalidCleaner,
        );

        let transcript = Transcript {
            text: "  hello from TinyVox  "
                .to_string(),
        };

        let result =
            pipeline.clean(&transcript).await;

        assert_eq!(
            result.text,
            "hello from TinyVox"
        );
    }
}