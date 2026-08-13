use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BLACK_BRUSH, BeginPaint, CreateRoundRectRgn, DeleteObject, DrawTextW, EndPaint,
            FillRgn, GetStockObject, HBRUSH, PAINTSTRUCT, SetBkMode, SetTextColor, TRANSPARENT,
        },
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
            DispatchMessageW, GetSystemMetrics, HWND_TOPMOST, LWA_ALPHA, MSG, PM_REMOVE,
            PeekMessageW, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
            SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            TranslateMessage, WM_APP, WM_DESTROY, WM_NCCREATE, WM_PAINT, WM_QUIT, WNDCLASSW,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::w,
};

const WM_OVERLAY_SHOW: u32 = WM_APP + 10;
const WM_OVERLAY_HIDE: u32 = WM_APP + 11;
const WM_OVERLAY_SHUTDOWN: u32 = WM_APP + 12;
const WM_OVERLAY_STATE: u32 = WM_APP + 13;

const OVERLAY_WIDTH: i32 = 220;
const OVERLAY_HEIGHT: i32 = 48;
const OVERLAY_RADIUS: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Recording,
    Transcribing,
    Cleaning,
    Injecting,
    Busy,
}

impl OverlayState {
    fn text(self) -> &'static str {
        match self {
            Self::Recording => "Recording",
            Self::Transcribing => "Transcribing",
            Self::Cleaning => "Cleaning",
            Self::Injecting => "Injecting",
            Self::Busy => "Busy",
        }
    }

    fn as_u32(self) -> u32 {
        match self {
            Self::Recording => 0,
            Self::Transcribing => 1,
            Self::Cleaning => 2,
            Self::Injecting => 3,
            Self::Busy => 4,
        }
    }

    fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Recording),
            1 => Some(Self::Transcribing),
            2 => Some(Self::Cleaning),
            3 => Some(Self::Injecting),
            4 => Some(Self::Busy),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum OverlayError {
    WindowCreationFailed,
    WindowClassRegistrationFailed,
    ThreadStartup,
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WindowCreationFailed => {
                write!(f, "failed to create overlay window")
            }

            Self::WindowClassRegistrationFailed => {
                write!(f, "failed to register overlay window class")
            }

            Self::ThreadStartup => {
                write!(f, "overlay thread failed to start")
            }
        }
    }
}

impl std::error::Error for OverlayError {}

enum OverlayCommand {
    Show,
    Hide,
    SetState(OverlayState),
    Shutdown,
}

struct OverlayData {
    state: OverlayState,
}

pub struct WindowsOverlay {
    thread: Option<JoinHandle<()>>,
    sender: Sender<OverlayCommand>,
}

impl WindowsOverlay {
    pub fn new() -> Result<Self, OverlayError> {
        let (startup_tx, startup_rx) = mpsc::channel();

        let (command_tx, command_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            overlay_thread(startup_tx, command_rx);
        });

        startup_rx
            .recv()
            .map_err(|_| OverlayError::ThreadStartup)??;

        Ok(Self {
            thread: Some(thread),
            sender: command_tx,
        })
    }

    pub fn show(&self) {
        let _ = self.sender.send(OverlayCommand::Show);
    }

    pub fn hide(&self) {
        let _ = self.sender.send(OverlayCommand::Hide);
    }

    pub fn set_state(&self, state: OverlayState) {
        let _ = self.sender.send(OverlayCommand::SetState(state));
    }
}

impl Drop for WindowsOverlay {
    fn drop(&mut self) {
        let _ = self.sender.send(OverlayCommand::Shutdown);

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn overlay_thread(
    startup_tx: Sender<Result<(), OverlayError>>,
    command_rx: Receiver<OverlayCommand>,
) {
    let class_name = w!("TinyVoxOverlay");

    let h_instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .unwrap_or_default();

    let class = WNDCLASSW {
        lpfnWndProc: Some(overlay_window_proc),
        hInstance: h_instance.into(),
        lpszClassName: class_name,
        style: CS_HREDRAW | CS_VREDRAW,
        hbrBackground: unsafe { HBRUSH(GetStockObject(BLACK_BRUSH).0) },
        ..Default::default()
    };

    let registered = unsafe { RegisterClassW(&class) };

    if registered == 0 {
        let _ = startup_tx.send(Err(OverlayError::WindowClassRegistrationFailed));

        return;
    }

    let data = Box::new(OverlayData {
        state: OverlayState::Recording,
    });

    let data_ptr = Box::into_raw(data);

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            class_name,
            w!("TinyVox"),
            WS_POPUP,
            0,
            0,
            OVERLAY_WIDTH,
            OVERLAY_HEIGHT,
            None,
            None,
            Some(h_instance.into()),
            Some(data_ptr as *const _),
        )
    };

    let hwnd = match hwnd {
        Ok(hwnd) => hwnd,

        Err(_) => {
            unsafe {
                drop(Box::from_raw(data_ptr));
            }

            let _ = startup_tx.send(Err(OverlayError::WindowCreationFailed));

            return;
        }
    };

    unsafe {
        let _ = SetLayeredWindowAttributes(
            hwnd,
            windows::Win32::Foundation::COLORREF(0),
            255,
            LWA_ALPHA,
        );
    }

    position_overlay(hwnd);

    let _ = startup_tx.send(Ok(()));

    let mut message = MSG::default();

    loop {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                OverlayCommand::Show => {
                    position_overlay(hwnd);

                    unsafe {
                        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                }

                OverlayCommand::Hide => unsafe {
                    ShowWindow(hwnd, SW_HIDE);
                },

                OverlayCommand::SetState(state) => unsafe {
                    let ptr = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                    );

                    if ptr != 0 {
                        let data = &mut *(ptr as *mut OverlayData);

                        data.state = state;
                    }
                },

                OverlayCommand::Shutdown => {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }

                    return;
                }
            }
        }

        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                return;
            }

            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn position_overlay(hwnd: HWND) {
    let screen_width =
        unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN) };

    let x = (screen_width - OVERLAY_WIDTH) / 2;

    let y = 32;

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn overlay_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create_struct = unsafe {
                &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW)
            };

            let data = create_struct.lpCreateParams;

            unsafe {
                SetWindowLongPtrW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                    data as isize,
                );
            }

            LRESULT(1)
        }

        WM_PAINT => {
            unsafe {
                paint_overlay(hwnd);
            }

            LRESULT(0)
        }

        WM_DESTROY => {
            let ptr = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                )
            };

            if ptr != 0 {
                unsafe {
                    SetWindowLongPtrW(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                        0,
                    );

                    drop(Box::from_raw(ptr as *mut OverlayData));
                }
            }

            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe fn paint_overlay(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();

    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };

    let ptr = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
        )
    };

    if ptr == 0 {
        unsafe {
            EndPaint(hwnd, &paint);
        }

        return;
    }

    let data = unsafe { &*(ptr as *const OverlayData) };

    let region = unsafe {
        CreateRoundRectRgn(
            0,
            0,
            OVERLAY_WIDTH,
            OVERLAY_HEIGHT,
            OVERLAY_RADIUS,
            OVERLAY_RADIUS,
        )
    };

    let brush = unsafe {
        windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(
            0x001E1E1E,
        ))
    };

    unsafe {
        FillRgn(hdc, region, brush);
    }

    let text = data.state.text();

    let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: OVERLAY_WIDTH,
        bottom: OVERLAY_HEIGHT,
    };

    unsafe {
        SetBkMode(hdc, TRANSPARENT);

        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));

        DrawTextW(
            hdc,
            &mut wide,
            &mut rect,
            windows::Win32::Graphics::Gdi::DT_CENTER
                | windows::Win32::Graphics::Gdi::DT_VCENTER
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );

        let _ = DeleteObject(region.into());

        let _ = DeleteObject(brush.into());

        EndPaint(hwnd, &paint);
    }
}
