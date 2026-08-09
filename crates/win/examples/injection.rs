use std::io::{self, Write};

use clipboard_win::get_clipboard_string;
use tinyvox_engine::ports::{CleanedText, TextInjector};
use win::WindowsTextInjector;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TinyVox — Text Injection");
    println!("========================\n");

    let clipboard_text = get_clipboard_string()
    .map_err(|e| format!("failed to read clipboard: {e:?}"))?;

    if clipboard_text.is_empty() {
        println!("Clipboard is empty.");
        return Ok(());
    }

    println!("Clipboard text:");
    println!("----------------");
    println!("{clipboard_text}");
    println!("----------------\n");

    let text = CleanedText {
        text: clipboard_text,
    };

    let injector = WindowsTextInjector::new();

    println!("Switch to a text field.");
    println!("Injection starts in 3 seconds...");

    for i in (1..=3).rev() {
        println!("{i}...");
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    injector.inject(&text)?;

    println!("Injection complete.");

    print!("Press ENTER to exit...");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(())
}