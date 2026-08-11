use std::ptr;

use windows::{
    core::{PCWSTR, PWSTR},
    Win32::Security::Credentials::{
        CredFree,
        CredReadW,
        CredWriteW,
        CREDENTIALW,
        CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    },
};

const TARGET_PREFIX: &str = "TinyVox";

#[derive(Debug)]
pub enum CredentialError {
    Write(windows::core::Error),
    Read(windows::core::Error),
    InvalidUtf8,
}

impl std::fmt::Display for CredentialError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Write(error) => {
                write!(
                    f,
                    "failed to write credential: {error}"
                )
            }

            Self::Read(error) => {
                write!(
                    f,
                    "failed to read credential: {error}"
                )
            }

            Self::InvalidUtf8 => {
                write!(
                    f,
                    "credential contains invalid UTF-8"
                )
            }
        }
    }
}

impl std::error::Error for CredentialError {}

pub struct WindowsCredentials;

impl WindowsCredentials {
    pub fn new() -> Self {
        Self
    }

    fn target_name(name: &str) -> String {
        format!("{TARGET_PREFIX}:{name}")
    }

    pub fn store(
        &self,
        name: &str,
        value: &str,
    ) -> Result<(), CredentialError> {
        let target = Self::target_name(name);

        let target_wide: Vec<u16> =
            target.encode_utf16().chain(Some(0)).collect();

        let value_bytes = value.as_bytes();

        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(
                target_wide.as_ptr() as *mut u16,
            ),
            CredentialBlobSize: value_bytes.len() as u32,
            CredentialBlob: value_bytes.as_ptr()
                as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };

        unsafe {
            CredWriteW(
                &credential,
                0,
            )
        }
        .map_err(CredentialError::Write)?;

        Ok(())
    }

    pub fn load(
        &self,
        name: &str,
    ) -> Result<String, CredentialError> {
        let target = Self::target_name(name);

        let target_wide: Vec<u16> =
            target.encode_utf16().chain(Some(0)).collect();

        let mut credential: *mut CREDENTIALW =
            ptr::null_mut();

        unsafe {
            CredReadW(
                PCWSTR(target_wide.as_ptr()),
                CRED_TYPE_GENERIC,
                Some(0),
                &mut credential,
            )
        }
        .map_err(CredentialError::Read)?;

        if credential.is_null() {
            return Err(
                CredentialError::Read(
                    windows::core::Error::empty(),
                ),
            );
        }

        let result = unsafe {
            let credential_ref = &*credential;

            let bytes = std::slice::from_raw_parts(
                credential_ref.CredentialBlob,
                credential_ref.CredentialBlobSize as usize,
            );

            String::from_utf8(bytes.to_vec())
                .map_err(|_| CredentialError::InvalidUtf8)
        };

        unsafe {
            CredFree(credential as *const _);
        }

        result
    }
}

impl Default for WindowsCredentials {
    fn default() -> Self {
        Self::new()
    }
}