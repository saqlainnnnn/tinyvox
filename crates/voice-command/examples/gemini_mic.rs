use std::time::Duration;

use audio::CpalAudioStreamer;
use dotenvy::dotenv;
use tokio::time::{
    sleep,
    timeout,
};
use voice_command::{
    GeminiLiveProvider,
    VoiceEvent,
    VoiceProvider,
    VoiceSession,
};

#[tokio::main]
async fn main()
    -> Result<(), Box<dyn std::error::Error>>
{
    dotenv().ok();

    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(
            |_| "failed to install rustls crypto provider",
        )?;

    println!(
        "🔌 Connecting to Gemini Live..."
    );

    let provider =
        GeminiLiveProvider::from_env()?;

    let mut session =
        provider.connect().await?;

    println!(
        "✓ Gemini Live session ready."
    );

    let mut microphone =
        CpalAudioStreamer::new()?;

    microphone.start()?;

    println!(
        "🎙️ Speak for 10 seconds..."
    );

    let start =
        std::time::Instant::now();

    let mut chunks_sent = 0usize;
    let mut bytes_sent = 0usize;

    while start.elapsed()
        < Duration::from_secs(10)
    {
        let chunk =
            microphone.read_chunk()?;

        if !chunk.is_empty() {
            let size = chunk.len();

            session
                .send_audio(
                    voice_command::AudioChunk {
                        samples: chunk,
                    },
                )
                .await?;

            chunks_sent += 1;
            bytes_sent += size;

            println!(
                "→ Sent microphone chunk: {} bytes",
                size
            );
        }

        while let Some(event) =
            poll_without_blocking(
                &mut session,
            )
            .await?
        {
            match event {
                VoiceEvent::AudioOut(chunk) => {
                    println!(
                        "← Gemini audio: {} bytes",
                        chunk.samples.len()
                    );
                }

                VoiceEvent::TurnComplete => {
                    println!(
                        "✓ Turn complete."
                    );
                }

                VoiceEvent::Error(error) => {
                    eprintln!(
                        "Gemini error: {error}"
                    );
                }

                _ => {}
            }
        }

        sleep(
            Duration::from_millis(40),
        )
        .await;
    }

    microphone.stop();

    println!(
        "✓ Microphone stream stopped."
    );

    println!(
        "✓ Sent {} chunks / {} bytes to Gemini.",
        chunks_sent,
        bytes_sent
    );

    Ok(())
}

async fn poll_without_blocking<V>(
    session: &mut V,
) -> Result<
    Option<VoiceEvent>,
    Box<dyn std::error::Error>,
>
where
    V: VoiceSession,
    V::Error: std::error::Error + 'static,
{
    timeout(
        Duration::from_millis(1),
        session.poll_event(),
    )
    .await
    .ok()
    .transpose()
    .map_err(Into::into)
}