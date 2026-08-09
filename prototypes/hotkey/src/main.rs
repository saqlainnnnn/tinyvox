use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Sender},
    OnceLock,
};
use std::thread;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
};

const VK_F9: u32 = 0x78;

static EVENT_SENDER: OnceLock<Sender<HotkeyEvent>> = OnceLock::new();
static RECORDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
enum HotkeyEvent {
    KeyDown,
    KeyUp,
}

fn main() {
    println!("TinyVox — Hotkey PoC");
    println!("Hold F9 to simulate dictation.");
    println!("Press Ctrl+C to exit.\n");

    let (tx, rx) = mpsc::channel::<HotkeyEvent>();

    EVENT_SENDER
        .set(tx)
        .expect("Failed to initialize global event sender");

    let hook_thread = thread::spawn(|| unsafe {
        let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) {
            Ok(hook) => hook,
            Err(error) => {
                eprintln!("Failed to install keyboard hook: {error}");
                return;
            }
        };

        println!("Keyboard hook installed.");
        println!("Listening for F9...\n");

        let mut msg = MSG::default();

        while GetMessageW(&mut msg, None, 0, 0).into() {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWindowsHookEx(hook);

        println!("Keyboard hook removed.");
    });

    while let Ok(event) = rx.recv() {
        match event {
            HotkeyEvent::KeyDown => {
                println!("[F9] Recording started");
            }

            HotkeyEvent::KeyUp => {
                println!("[F9] Recording stopped");
            }
        }
    }

    let _ = hook_thread.join();
}

unsafe extern "system" fn keyboard_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let keyboard = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        if keyboard.vkCode == VK_F9 {
            match wparam.0 as u32 {
                WM_KEYDOWN => {
                    // Ignore auto-repeat keydown events.
                    if !RECORDING.swap(true, Ordering::AcqRel) {
                        if let Some(sender) = EVENT_SENDER.get() {
                            let _ = sender.send(HotkeyEvent::KeyDown);
                        }
                    }
                }

                WM_KEYUP => {
                    if RECORDING.swap(false, Ordering::AcqRel) {
                        if let Some(sender) = EVENT_SENDER.get() {
                            let _ = sender.send(HotkeyEvent::KeyUp);
                        }
                    }
                }

                _ => {}
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}