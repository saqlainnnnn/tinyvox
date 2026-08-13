use std::thread;
use std::time::Duration;

use clipboard_win::{ErrorCode, get_clipboard_string, set_clipboard_string};

use tinyvox_engine::ports::{CleanedText, TextInjector};

use windows::Win32::{
    Foundation::HWND,
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_V,
        },
        WindowsAndMessaging::SetForegroundWindow,
    },
};

use crate::foreground::WindowsForeground;

const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub enum InjectionError {
    ClipboardRead(ErrorCode),
    ClipboardWrite(ErrorCode),
    SendInputFailed,
    ForegroundUnavailable,
}

impl std::fmt::Display for InjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClipboardRead(error) => {
                write!(f, "failed to read clipboard: {error}")
            }

            Self::ClipboardWrite(error) => {
                write!(f, "failed to write clipboard: {error}")
            }

            Self::SendInputFailed => {
                write!(f, "Windows SendInput failed")
            }
            Self::ForegroundUnavailable => {
                write!(f, "failed to capture foreground window")
            }
        }
    }
}

impl std::error::Error for InjectionError {}

pub struct WindowsTextInjector {
    target: Option<HWND>,
}

impl WindowsTextInjector {
    pub fn new() -> Self {
        Self { target: None }
    }
    pub fn set_target(&mut self, target: HWND) {
        self.target = Some(target);
    }
    pub fn clear_target(&mut self) {
        self.target = None;
    }

    fn send_ctrl_v() -> Result<(), InjectionError> {
        let inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_V,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_V,
                        dwFlags: KEYEVENTF_KEYUP,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL,
                        dwFlags: KEYEVENTF_KEYUP,
                        ..Default::default()
                    },
                },
            },
        ];

        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };

        if sent != inputs.len() as u32 {
            return Err(InjectionError::SendInputFailed);
        }

        Ok(())
    }
}

impl Default for WindowsTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for WindowsTextInjector {
    type Error = InjectionError;

    fn inject(&self, text: &CleanedText) -> Result<(), Self::Error> {
        if let Some(hwnd) = self.target {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
        }

        let previous_clipboard = get_clipboard_string().map_err(InjectionError::ClipboardRead)?;

        let previous_clipboard = get_clipboard_string().map_err(InjectionError::ClipboardRead)?;

        set_clipboard_string(&text.text).map_err(InjectionError::ClipboardWrite)?;

        let injection_result = Self::send_ctrl_v();

        /*
         * SendInput queues the keyboard events.
         * Give the foreground application a small
         * amount of time to process Ctrl+V before
         * restoring the clipboard.
         */
        thread::sleep(CLIPBOARD_RESTORE_DELAY);

        let restore_result =
            set_clipboard_string(&previous_clipboard).map_err(InjectionError::ClipboardWrite);

        injection_result?;
        restore_result?;

        Ok(())
    }

    fn prepare(&mut self) -> Result<(), Self::Error> {
        let foreground = WindowsForeground::new();

        let window = foreground
            .get()
            .map_err(|_| InjectionError::ForegroundUnavailable)?;

        self.target = Some(window.hwnd);

        Ok(())
    }
}
