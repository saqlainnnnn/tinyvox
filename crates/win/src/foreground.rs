use windows::Win32::{
    Foundation::HWND,
    System::{
        ProcessStatus::GetModuleFileNameExW,
        Threading::{
            OpenProcess,
            PROCESS_QUERY_INFORMATION,
            PROCESS_VM_READ,
        },
    },
    UI::WindowsAndMessaging::{
        GetForegroundWindow,
        GetWindowThreadProcessId,
    },
};

#[derive(Debug, Clone)]
pub struct ForegroundWindow {
    pub hwnd: HWND,
    pub process_name: String,
}

#[derive(Debug)]
pub enum ForegroundError {
    NoForegroundWindow,
    ProcessIdUnavailable,
    ProcessOpenFailed,
    ProcessNameUnavailable,
}

impl std::fmt::Display for ForegroundError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::NoForegroundWindow => {
                write!(f, "no foreground window found")
            }

            Self::ProcessIdUnavailable => {
                write!(f, "failed to get foreground process ID")
            }

            Self::ProcessOpenFailed => {
                write!(f, "failed to open foreground process")
            }

            Self::ProcessNameUnavailable => {
                write!(
                    f,
                    "failed to get foreground process name"
                )
            }
        }
    }
}

impl std::error::Error for ForegroundError {}

pub struct WindowsForeground;

impl WindowsForeground {
    pub fn new() -> Self {
        Self
    }

    pub fn get(
        &self,
    ) -> Result<ForegroundWindow, ForegroundError> {
        let hwnd = unsafe {
            GetForegroundWindow()
        };

        if hwnd.0.is_null() {
            return Err(
                ForegroundError::NoForegroundWindow
            );
        }

        let mut process_id = 0u32;

        unsafe {
            GetWindowThreadProcessId(
                hwnd,
                Some(&mut process_id),
            );
        }

        if process_id == 0 {
            return Err(
                ForegroundError::ProcessIdUnavailable
            );
        }

        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_READ,
                false,
                process_id,
            )
        }
        .map_err(|_| {
            ForegroundError::ProcessOpenFailed
        })?;

        let mut buffer = [0u16; 260];

        let length = unsafe {
            GetModuleFileNameExW(
                Some(process),
                None,
                &mut buffer,
            )
        };

        if length == 0 {
            return Err(
                ForegroundError::ProcessNameUnavailable
            );
        }

        let process_name =
            String::from_utf16_lossy(
                &buffer[..length as usize],
            );

        let process_name = process_name
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&process_name)
            .to_string();

        Ok(ForegroundWindow {
            hwnd,
            process_name,
        })
    }
}

impl Default for WindowsForeground {
    fn default() -> Self {
        Self::new()
    }
}