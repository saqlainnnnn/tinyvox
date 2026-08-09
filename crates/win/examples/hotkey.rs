use win::{HotkeyEvent, WindowsHotkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TinyVox Windows Hotkey");
    println!("Hold F9 to generate events.");
    println!("Press Ctrl+C to exit.\n");

    let _hotkey = WindowsHotkey::new()?;

    loop {
        match _hotkey.recv()? {
            HotkeyEvent::Pressed => {
                println!("F9 pressed");
            }

            HotkeyEvent::Released => {
                println!("F9 released");
            }
        }
    }
}