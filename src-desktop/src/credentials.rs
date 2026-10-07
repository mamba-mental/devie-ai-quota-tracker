//! Token storage under the app data folder. Windows uses current-user DPAPI
//! encryption; Unix keeps private JSON files (`0600`) without Keychain prompts.

use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Credentials {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Provider-specific data, such as the ChatGPT account id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Provider-specific project data, such as a Gemini Code Assist project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl Credentials {
    /// True when the access token expires within `lead`.
    pub fn expires_within(&self, lead: Duration) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at - Utc::now() <= lead)
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = self.refresh_token.as_mut() {
            token.zeroize();
        }
    }
}

fn path_for(app_data_dir: &Path, connection_id: &str) -> PathBuf {
    #[cfg(windows)]
    let extension = "dpapi";
    #[cfg(not(windows))]
    let extension = "json";
    app_data_dir
        .join("credentials")
        .join(format!("{connection_id}.{extension}"))
}

pub fn load(app_data_dir: &Path, connection_id: &str) -> Result<Credentials, String> {
    let data = Zeroizing::new(
        fs::read(path_for(app_data_dir, connection_id)).map_err(|_| {
            "The saved login for this account is missing. Sign in again.".to_string()
        })?,
    );
    #[cfg(windows)]
    let data = dpapi::unprotect(&data)?;
    serde_json::from_slice(&data)
        .map_err(|_| "The saved login for this account is invalid. Sign in again.".to_string())
}

pub fn save(
    app_data_dir: &Path,
    connection_id: &str,
    credentials: &Credentials,
) -> Result<(), String> {
    let path = path_for(app_data_dir, connection_id);
    let parent = path
        .parent()
        .ok_or_else(|| "The credentials folder is invalid.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let bytes =
        Zeroizing::new(serde_json::to_vec_pretty(credentials).map_err(|error| error.to_string())?);
    #[cfg(windows)]
    let bytes = dpapi::protect(&bytes)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, &*bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

pub fn remove(app_data_dir: &Path, connection_id: &str) {
    let _ = fs::remove_file(path_for(app_data_dir, connection_id));
}

#[cfg(windows)]
mod dpapi {
    use std::{ptr, slice};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    use zeroize::{Zeroize, Zeroizing};

    // DPAPI owns this allocation. Always erase and release it, including error paths.
    struct Output(CRYPT_INTEGER_BLOB);

    impl Drop for Output {
        fn drop(&mut self) {
            if !self.0.pbData.is_null() {
                // SAFETY: DPAPI returns cbData initialized bytes allocated with LocalAlloc.
                unsafe {
                    slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                    LocalFree(self.0.pbData.cast());
                }
            }
        }
    }

    fn transform(data: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>, String> {
        // Own a mutable copy because the native BLOB type exposes a mutable pointer.
        let mut input = Zeroizing::new(data.to_vec());
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: input
                .len()
                .try_into()
                .map_err(|_| "The saved login is too large.".to_string())?,
            pbData: input.as_mut_ptr(),
        };
        let mut output = Output(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        });
        // SAFETY: input and output remain valid for the synchronous call. No optional
        // pointers are supplied. Omitting LOCAL_MACHINE binds protection to this user.
        let success = unsafe {
            if encrypt {
                CryptProtectData(
                    &input_blob,
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output.0,
                )
            } else {
                CryptUnprotectData(
                    &input_blob,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output.0,
                )
            }
        };
        if success == 0 || output.0.pbData.is_null() {
            return Err(if encrypt {
                "Windows could not protect the saved login."
            } else {
                "Windows could not unlock the saved login. Sign in again."
            }
            .to_string());
        }
        // SAFETY: successful DPAPI call initialized cbData bytes; Output retains ownership.
        Ok(Zeroizing::new(unsafe {
            slice::from_raw_parts(output.0.pbData, output.0.cbData as usize).to_vec()
        }))
    }

    pub(super) fn protect(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        transform(data, true)
    }

    pub(super) fn unprotect(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        transform(data, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_encrypts_overwrites_and_removes_credentials() {
        let root = tempfile::tempdir().expect("temp dir");
        let mut credentials = Credentials {
            access_token: "synthetic-access-secret-12345".into(),
            refresh_token: Some("synthetic-refresh-secret-67890".into()),
            expires_at: None,
            account_id: None,
            project_id: None,
        };
        save(root.path(), "windows-test", &credentials).expect("save encrypted");
        let path = path_for(root.path(), "windows-test");
        let disk = fs::read(&path).expect("read encrypted file");
        assert!(serde_json::from_slice::<Credentials>(&disk).is_err());
        for secret in [
            &credentials.access_token,
            credentials.refresh_token.as_ref().unwrap(),
        ] {
            assert!(!disk
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()));
        }
        assert_eq!(
            load(root.path(), "windows-test").expect("decrypt"),
            credentials
        );
        credentials.access_token = "synthetic-replacement-secret".into();
        save(root.path(), "windows-test", &credentials).expect("overwrite encrypted");
        assert_eq!(
            load(root.path(), "windows-test").expect("decrypt replacement"),
            credentials
        );
        assert!(!path.with_extension("tmp").exists());
        remove(root.path(), "windows-test");
        assert!(!path.exists());
        assert!(load(root.path(), "windows-test").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_corrupted_ciphertext_and_plaintext() {
        let root = tempfile::tempdir().expect("temp dir");
        let credentials = Credentials {
            access_token: "synthetic-secret".into(),
            refresh_token: None,
            expires_at: None,
            account_id: None,
            project_id: None,
        };
        save(root.path(), "corrupt-test", &credentials).expect("save encrypted");
        let path = path_for(root.path(), "corrupt-test");
        let mut disk = fs::read(&path).expect("read encrypted file");
        let last = disk.len() - 1;
        disk[last] ^= 0xff;
        fs::write(&path, &disk).expect("write corrupted ciphertext");
        assert!(load(root.path(), "corrupt-test").is_err());
        fs::write(&path, &disk[..disk.len() / 2]).expect("write truncated ciphertext");
        assert!(load(root.path(), "corrupt-test").is_err());
        fs::write(&path, serde_json::to_vec(&credentials).unwrap())
            .expect("write synthetic plaintext");
        assert!(load(root.path(), "corrupt-test").is_err());
    }

    #[test]
    fn round_trips_and_restricts_permissions() {
        let root = tempfile::tempdir().expect("temp dir");
        let credentials = Credentials {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            account_id: None,
            project_id: Some("project-123".into()),
        };
        save(root.path(), "abc", &credentials).expect("save");
        let loaded = load(root.path(), "abc").expect("load");
        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.project_id.as_deref(), Some("project-123"));
        assert!(!loaded.expires_within(Duration::minutes(5)));
        assert!(loaded.expires_within(Duration::hours(2)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path_for(root.path(), "abc"))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        remove(root.path(), "abc");
        assert!(load(root.path(), "abc").is_err());
    }
}
