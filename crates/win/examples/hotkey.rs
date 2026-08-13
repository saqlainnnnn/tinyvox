use win::{HotkeyEvent, WindowsHotkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TinyVox Windows Hotkey");
    println!("Hold F9 to generate dictation events.");
    println!("Press F10 to toggle voice-command mode.");
    println!("Press Ctrl+C to exit.\n");

    let hotkey = WindowsHotkey::new()?;

    loop {
        match hotkey.recv()? {
            HotkeyEvent::Pressed => {
                println!("🎙️ F9 pressed");
            }

            HotkeyEvent::Released => {
                println!("⏹️ F9 released");
            }

            HotkeyEvent::VoiceCommandToggled => {
                println!("🎧 F10 voice-command toggle");
            }
        }
    }
}
