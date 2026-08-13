use std::time::Duration;

use audio::{CpalAudioPlayback, CpalAudioStreamer};

use dotenvy::dotenv;

use tokio::time::sleep;

use voice_command::{
    AudioChunk, GeminiLiveProvider, VoiceEvent, VoiceProvider,
    gemini::{GeminiReceiveHandle, GeminiSendHandle},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenv().ok();

    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| "failed to install rustls crypto provider")?;
    }

    println!("🔌 Connecting to Gemini Live...");

    let provider = GeminiLiveProvider::from_env()?;

    let session = provider.connect().await?;

    println!("✓ Gemini Live session ready.");

    let (send_handle, receive_handle) = session.split();

    let sender = tokio::spawn(microphone_task(send_handle));

    let receiver = tokio::spawn(speaker_task(receive_handle));

    let (chunks_sent, bytes_sent) = sender.await??;

    println!(
        "✓ Sent {} microphone chunks / {} bytes.",
        chunks_sent, bytes_sent,
    );

    let (audio_bytes, turns) = receiver.await??;

    println!(
        "✓ Received {} bytes of Gemini audio across {} turns.",
        audio_bytes, turns,
    );

    Ok(())
}

async fn microphone_task(
    send_handle: GeminiSendHandle,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    let mut microphone = CpalAudioStreamer::new()?;

    microphone.start()?;

    println!("🎙️ Speak for 10 seconds...");

    let start = std::time::Instant::now();

    let mut chunks_sent = 0usize;
    let mut bytes_sent = 0usize;

    while start.elapsed() < Duration::from_secs(10) {
        let chunk = microphone.read_chunk()?;

        if !chunk.is_empty() {
            let size = chunk.len();

            send_handle
                .send_audio(AudioChunk { samples: chunk })
                .await?;

            chunks_sent += 1;
            bytes_sent += size;
        }

        sleep(Duration::from_millis(40)).await;
    }

    microphone.stop();

    send_handle.end_audio().await?;

    println!("✓ Microphone stream stopped.");

    println!("→ Sent audioStreamEnd to Gemini.");

    Ok((chunks_sent, bytes_sent))
}

async fn speaker_task(
    mut receive_handle: GeminiReceiveHandle,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    let mut playback = CpalAudioPlayback::new()?;

    playback.start()?;

    println!("🔊 Speaker playback ready.");

    let mut audio_bytes = 0usize;
    let mut turns = 0usize;

    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), receive_handle.poll_event())
            .await
            .map_err(|_| "timed out waiting for Gemini response")??;

        match event {
            VoiceEvent::AudioOut(chunk) => {
                audio_bytes += chunk.samples.len();

                println!("← Gemini audio: {} bytes", chunk.samples.len());

                playback.push_pcm16(&chunk.samples)?;
            }

            VoiceEvent::TurnComplete => {
                turns += 1;

                println!("✓ Turn complete.");

                break;
            }

            VoiceEvent::Error(error) => {
                playback.stop();

                return Err(format!("Gemini error: {error}").into());
            }

            _ => {}
        }
    }

    /*
     * Give CPAL time to drain queued audio
     * before destroying the stream.
     */
    sleep(Duration::from_secs(2)).await;

    playback.stop();

    println!("✓ Speaker playback finished.");

    Ok((audio_bytes, turns))
}
