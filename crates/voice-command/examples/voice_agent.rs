use dotenvy::dotenv;

use tinyvox_engine::{
    dictionary::shared as shared_dictionary, last_dictation::shared as shared_last_dictation,
    tool_registry::ToolRegistry,
};

use voice_command::{GeminiLiveProvider, VoiceAgent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenv().ok();

    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| "failed to install rustls crypto provider")?;
    }

    let dictionary = shared_dictionary();

    let last_dictation = shared_last_dictation();

    {
        let mut last = last_dictation.write().unwrap();

        last.replace("This is a real TinyVox voice agent test.");
    }

    let tool_registry = ToolRegistry::new(dictionary, last_dictation);

    let provider = GeminiLiveProvider::from_env()?;

    let mut agent = VoiceAgent::new(provider, tool_registry);

    agent.start().await?;

    println!();
    println!("🎧 Voice agent is running.");
    println!("Speak normally. Say something like:");
    println!("\"What was my last dictation?\"");
    println!("Press ENTER here to stop.");

    let _ = tokio::task::spawn_blocking(|| {
        let mut input = String::new();

        let _ = std::io::stdin().read_line(&mut input);
    })
    .await;

    agent.stop().await?;

    Ok(())
}
