use dotenvy::dotenv;
use serde_json::{
    json,
    Value,
};

use tinyvox_engine::{
    dictionary::shared as shared_dictionary,
    last_dictation::shared as shared_last_dictation,
    tool_registry::ToolRegistry,
    tools::ToolRequest,
};

use voice_command::{
    GeminiLiveProvider,
    VoiceEvent,
    VoiceProvider,
    gemini::{
        GeminiReceiveHandle,
        GeminiSendHandle,
    },
};

#[tokio::main]
async fn main()
    -> Result<
        (),
        Box<dyn std::error::Error + Send + Sync>,
    >
{
    dotenv().ok();

    if rustls::crypto::CryptoProvider::get_default()
        .is_none()
    {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(
                |_| {
                    "failed to install rustls crypto provider"
                },
            )?;
    }

    // ---------------------------------------------------------
    // Build real engine tool state.
    // ---------------------------------------------------------

    let dictionary =
        shared_dictionary();

    let last_dictation =
        shared_last_dictation();

    {
        let mut last =
            last_dictation
                .write()
                .unwrap();

        last.replace(
            "Hello from the TinyVox tool test",
        );
    }

    let mut registry =
        ToolRegistry::new(
            dictionary,
            last_dictation,
        );

    // ---------------------------------------------------------
    // Gemini
    // ---------------------------------------------------------

    let provider =
        GeminiLiveProvider::from_env()?;

    let session =
        provider.connect().await?;

    println!(
        "✓ Gemini Live session ready."
    );

    let (
        send_handle,
        receive_handle,
    ) = session.split();

    let receiver =
        tokio::spawn(
            receive_loop(
                send_handle,
                receive_handle,
                registry,
            ),
        );

    // Ask something that should require
    // read_last_dictation.
    //
    // We send this as realtime text so that
    // tool calling can be tested without the
    // microphone.
    //
    // The current Live API docs support realtime
    // text input through realtimeInput.text.
    let send_handle =
        GeminiLiveProvider::from_env()?;

    drop(send_handle);

    receiver.await??;

    Ok(())
}

async fn receive_loop(
    send_handle: GeminiSendHandle,
    mut receive_handle: GeminiReceiveHandle,
    mut registry: ToolRegistry,
) -> Result<
    (),
    Box<dyn std::error::Error + Send + Sync>,
> {
    send_handle
        .send_text(
            "What was my last dictation? Use the read_last_dictation tool to find out.",
        )
        .await?;

    loop {
        let event =
            tokio::time::timeout(
                std::time::Duration::from_secs(30),
                receive_handle.poll_event(),
            )
            .await
            .map_err(
                |_| {
                    "timed out waiting for Gemini"
                },
            )??;

        match event {
            VoiceEvent::ToolCall(
                tool_call,
            ) => {
                println!(
                    "🔧 Tool requested: {}",
                    tool_call.name,
                );

                println!(
                    "   args: {}",
                    tool_call.arguments,
                );

                let args:
                    Value =
                    serde_json::from_str(
                        &tool_call.arguments,
                    )?;

                let request =
                    match tool_call.name.as_str() {
                        "read_last_dictation" => {
                            ToolRequest::
                                ReadLastDictation
                        }

                        "add_dictionary_entry" => {
                            let wrong =
                                args
                                    .get("wrong")
                                    .and_then(
                                        Value::as_str,
                                    )
                                    .ok_or(
                                        "missing 'wrong' argument",
                                    )?;

                            let correct =
                                args
                                    .get("correct")
                                    .and_then(
                                        Value::as_str,
                                    )
                                    .ok_or(
                                        "missing 'correct' argument",
                                    )?;

                            ToolRequest::
                                AddDictionaryEntry {
                                    wrong:
                                        wrong.to_string(),
                                    correct:
                                        correct.to_string(),
                                }
                        }

                        unknown => {
                            send_handle
                                .send_tool_response(
                                    &tool_call.id,
                                    unknown,
                                    json!({
                                        "error":
                                            format!(
                                                "unknown tool: {unknown}"
                                            )
                                    }),
                                )
                                .await?;

                            continue;
                        }
                    };

                let result =
                    registry.execute(
                        request,
                    );

                println!(
                    "✓ Tool result: {:?}",
                    result,
                );

                let response =
                    serde_json::to_value(
                        &result,
                    )
                    .unwrap_or_else(
                        |_| {
                            json!({
                                "error":
                                    "failed to serialize tool result"
                            })
                        },
                    );

                send_handle
                    .send_tool_response(
                        &tool_call.id,
                        &tool_call.name,
                        json!({
                            "result":
                                response
                        }),
                    )
                    .await?;

                println!(
                    "→ Tool response sent."
                );
            }

            VoiceEvent::AudioOut(
                chunk,
            ) => {
                println!(
                    "← Gemini audio: {} bytes",
                    chunk.samples.len()
                );
            }

            VoiceEvent::TurnComplete => {
                println!(
                    "✓ Gemini finished the turn."
                );

                break;
            }

            VoiceEvent::Error(
                error,
            ) => {
                return Err(
                    format!(
                        "Gemini error: {error}"
                    )
                    .into(),
                );
            }

            _ => {}
        }
    }

    Ok(())
}