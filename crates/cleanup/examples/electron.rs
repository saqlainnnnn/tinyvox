use cleanup::ElectronCleaner;
use tinyvox_engine::ports::{TextCleaner, Transcript};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let cleaner = ElectronCleaner::from_env()?;

    let transcript = Transcript {
        text:
            "uh hey can you like remind me tomorrow to call john and tell him ill be there at five"
                .to_string(),
    };

    println!("Original:");
    println!("{}", transcript.text);
    println!();

    let cleaned = cleaner.clean(&transcript).await?;

    println!("Cleaned:");
    println!("{}", cleaned.text);

    Ok(())
}
