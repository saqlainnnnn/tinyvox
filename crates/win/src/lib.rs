pub mod credentials;
pub mod foreground;
pub mod injection;
pub mod overlay;

pub use credentials::{CredentialError, WindowsCredentials};

pub use foreground::{ForegroundError, ForegroundWindow, WindowsForeground};

pub use injection::{InjectionError, WindowsTextInjector};

pub use overlay::{OverlayError, OverlayState, WindowsOverlay};

use std::{
    cell::{Cell, RefCell},
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE,
        PeekMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP,
    },
};

const VK_F9: u32 = 0x78;
const VK_F10: u32 = 0x79;

const WM_TINYVOX_SHUTDOWN: u32 = WM_APP + 1;

thread_local! {
    static EVENT_SENDER:
        RefCell<Option<Sender<HotkeyEvent>>> =
        const {
            RefCell::new(None)
        };

    static F9_DOWN:
        Cell<bool> =
        const {
            Cell::new(false)
        };

    static F10_DOWN:
        Cell<bool> =
        const {
            Cell::new(false)
        };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// F9 key-down.
    Pressed,

    /// F9 key-up.
    Released,

    /// F10 toggles voice-command mode.
    VoiceCommandToggled,
}

#[derive(Debug)]
pub enum HotkeyError {
    HookInstallation(windows::core::Error),

    ThreadStartup,

    ThreadDisconnected,
}

impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HookInstallation(error) => {
                write!(f, "failed to install keyboard hook: {error}")
            }

            Self::ThreadStartup => {
                write!(f, "keyboard hook thread failed to start")
            }

            Self::ThreadDisconnected => {
                write!(f, "keyboard hook thread disconnected")
            }
        }
    }
}

impl std::error::Error for HotkeyError {}

pub struct WindowsHotkey {
    events: Receiver<HotkeyEvent>,

    thread_id: u32,

    thread: Option<JoinHandle<()>>,
}

impl WindowsHotkey {
    pub fn new() -> Result<Self, HotkeyError> {
        let (event_tx, event_rx) = mpsc::channel();

        let (startup_tx, startup_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            hook_thread(event_tx, startup_tx);
        });

        let thread_id = match startup_rx.recv() {
            Ok(Ok(thread_id)) => thread_id,

            Ok(Err(error)) => {
                let _ = thread.join();

                return Err(HotkeyError::HookInstallation(error));
            }

            Err(_) => {
                let _ = thread.join();

                return Err(HotkeyError::ThreadStartup);
            }
        };

        Ok(Self {
            events: event_rx,

            thread_id,

            thread: Some(thread),
        })
    }

    pub fn recv(&self) -> Result<HotkeyEvent, HotkeyError> {
        self.events
            .recv()
            .map_err(|_| HotkeyError::ThreadDisconnected)
    }

    pub fn try_recv(&self) -> Result<Option<HotkeyEvent>, HotkeyError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),

            Err(mpsc::TryRecvError::Empty) => Ok(None),

            Err(mpsc::TryRecvError::Disconnected) => Err(HotkeyError::ThreadDisconnected),
        }
    }

    pub fn shutdown(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_TINYVOX_SHUTDOWN, WPARAM(0), LPARAM(0));
        }

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WindowsHotkey {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn hook_thread(
    event_tx: Sender<HotkeyEvent>,

    startup_tx: Sender<Result<u32, windows::core::Error>>,
) {
    EVENT_SENDER.with(|sender| {
        *sender.borrow_mut() = Some(event_tx);
    });

    /*
     * Force creation of this thread's
     * Windows message queue.
     */
    let mut initial_message = MSG::default();

    unsafe {
        let _ = PeekMessageW(&mut initial_message, None, 0, 0, PM_NOREMOVE);
    }

    let thread_id = unsafe { GetCurrentThreadId() };

    let hook = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0) } {
        Ok(hook) => hook,

        Err(error) => {
            let _ = startup_tx.send(Err(error));

            return;
        }
    };

    let _ = startup_tx.send(Ok(thread_id));

    let mut message = MSG::default();

    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };

        if !result.as_bool() {
            break;
        }

        /*
         * This message only wakes the
         * message loop for shutdown.
         */
        if message.message == WM_TINYVOX_SHUTDOWN {
            break;
        }

        unsafe {
            let _ = TranslateMessage(&message);

            DispatchMessageW(&message);
        }
    }

    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }

    /*
     * Reset local hotkey state before
     * the thread terminates.
     */
    F9_DOWN.with(|down| {
        down.set(false);
    });

    F10_DOWN.with(|down| {
        down.set(false);
    });

    EVENT_SENDER.with(|sender| {
        *sender.borrow_mut() = None;
    });
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let keyboard = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };

        let message = wparam.0 as u32;

        match keyboard.vkCode {
            /*
             * F9 = hold-to-record
             */
            VK_F9 => {
                match message {
                    WM_KEYDOWN => {
                        F9_DOWN.with(|down| {
                            /*
                             * Ignore repeated
                             * WM_KEYDOWN events
                             * while held.
                             */
                            if !down.replace(true) {
                                send_event(HotkeyEvent::Pressed);
                            }
                        });
                    }

                    WM_KEYUP => {
                        F9_DOWN.with(|down| {
                            /*
                             * Only emit release
                             * if we previously
                             * emitted the press.
                             */
                            if down.replace(false) {
                                send_event(HotkeyEvent::Released);
                            }
                        });
                    }

                    _ => {}
                }
            }

            /*
             * F10 = voice-command toggle
             */
            VK_F10 => {
                match message {
                    WM_KEYDOWN => {
                        F10_DOWN.with(|down| {
                            /*
                             * F10 is a toggle,
                             * so emit exactly
                             * one event per
                             * physical press.
                             */
                            if !down.replace(true) {
                                send_event(HotkeyEvent::VoiceCommandToggled);
                            }
                        });
                    }

                    WM_KEYUP => {
                        F10_DOWN.with(|down| {
                            down.set(false);
                        });
                    }

                    _ => {}
                }
            }

            _ => {}
        }
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn send_event(event: HotkeyEvent) {
    EVENT_SENDER.with(|sender| {
        if let Some(sender) = sender.borrow().as_ref() {
            let _ = sender.send(event);
        }
    });
}
