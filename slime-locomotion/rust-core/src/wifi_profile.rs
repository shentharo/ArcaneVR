//! User-scoped Windows DPAPI storage. Only encrypted credentials reach disk.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, PartialEq, Eq)]
pub struct Profile {
    pub ssid: String,
    pub password: String,
    pub auto_setup: bool,
}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Profile")
            .field("ssid", &self.ssid)
            .field("password", &"[REDACTED]")
            .field("auto_setup", &self.auto_setup)
            .finish()
    }
}

fn location() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|s| !s.is_empty())
        .ok_or("The user data directory is unavailable.")?;
    let base = PathBuf::from(base);
    if !base.is_absolute() {
        return Err("The user data directory is invalid.".into());
    }
    Ok(base.join("ArcaneVR").join("wifi-profile.json"))
}

pub fn load() -> Result<Option<Profile>, String> {
    load_at(&location()?)
}
pub fn save(profile: &Profile) -> Result<(), String> {
    save_at(&location()?, profile)
}
pub fn forget() -> Result<(), String> {
    forget_at(&location()?)
}

/// Explicit file destination for isolated tests and user-selected storage.
pub fn load_at(path: &Path) -> Result<Option<Profile>, String> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not read the saved Wi-Fi profile.".into()),
    };
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the saved Wi-Fi profile.")?;
    if bytes.len() > 65_536 {
        return Err("The saved Wi-Fi profile is invalid.".into());
    }
    let outer: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "The saved Wi-Fi profile is invalid.")?;
    if outer.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("The saved Wi-Fi profile version is unsupported.".into());
    }
    let encrypted = outer
        .get("ciphertext")
        .and_then(|v| v.as_str())
        .ok_or("The saved Wi-Fi profile is invalid.")?;
    let blob = STANDARD
        .decode(encrypted)
        .map_err(|_| "The saved Wi-Fi profile is invalid.")?;
    let mut plain = platform::unprotect(&blob)?;
    let decoded = decode_profile(&plain);
    plain.fill(0);
    decoded.map(Some)
}

fn decode_profile(bytes: &[u8]) -> Result<Profile, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "The saved Wi-Fi profile is invalid.")?;
    let profile = Profile {
        ssid: value
            .get("ssid")
            .and_then(|v| v.as_str())
            .ok_or("The saved Wi-Fi profile is invalid.")?
            .into(),
        password: value
            .get("password")
            .and_then(|v| v.as_str())
            .ok_or("The saved Wi-Fi profile is invalid.")?
            .into(),
        auto_setup: value
            .get("auto_setup")
            .and_then(|v| v.as_bool())
            .ok_or("The saved Wi-Fi profile is invalid.")?,
    };
    crate::usb_setup::wifi_command(&profile.ssid, &profile.password)?;
    Ok(profile)
}

pub fn save_at(path: &Path, profile: &Profile) -> Result<(), String> {
    crate::usb_setup::wifi_command(&profile.ssid, &profile.password)?;
    let mut plain = serde_json::to_vec(&serde_json::json!({
        "ssid": profile.ssid, "password": profile.password, "auto_setup": profile.auto_setup,
    }))
    .map_err(|_| "Could not encode the Wi-Fi profile.")?;
    let protected = platform::protect(&plain);
    plain.fill(0);
    let protected = protected?;
    let encoded = serde_json::to_vec(
        &serde_json::json!({"version": 1, "ciphertext": STANDARD.encode(protected)}),
    )
    .map_err(|_| "Could not encode the protected Wi-Fi profile.")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("The Wi-Fi profile destination is invalid.")?;
    fs::create_dir_all(parent).map_err(|_| "Could not create the Wi-Fi profile directory.")?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut temporary = None;
    for _ in 0..32 {
        let name = format!(
            ".wifi-profile-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let temp_path = parent.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => {
                temporary = Some((temp_path, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Could not create the protected Wi-Fi profile.".into()),
        }
    }
    let (temp_path, mut file) = temporary.ok_or("Could not create the protected Wi-Fi profile.")?;
    let written = file.write_all(&encoded).and_then(|_| file.sync_all());
    drop(file);
    let result = written
        .map_err(|_| "Could not write the protected Wi-Fi profile.".to_string())
        .and_then(|_| platform::replace(&temp_path, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub fn forget_at(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not remove the saved Wi-Fi profile.".into()),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
    #[repr(C)]
    struct Blob {
        length: u32,
        data: *mut u8,
    }
    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    fn transform(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
        let length = u32::try_from(input.len()).map_err(|_| "The Wi-Fi profile is too large.")?;
        let input_blob = Blob {
            length,
            data: input.as_ptr().cast_mut(),
        };
        let mut output = Blob {
            length: 0,
            data: ptr::null_mut(),
        };
        // SAFETY: Win32 receives valid input bytes for the call's duration and owns the
        // output allocation. UI_FORBIDDEN=1; LOCAL_MACHINE is intentionally absent.
        let ok = unsafe {
            if encrypt {
                CryptProtectData(
                    &input_blob,
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    1,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input_blob,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    1,
                    &mut output,
                )
            }
        };
        if ok == 0 {
            return Err(if encrypt {
                "Windows could not protect the Wi-Fi profile."
            } else {
                "Windows could not unlock the saved Wi-Fi profile for this user."
            }
            .into());
        }
        if output.data.is_null() {
            return Err("Windows returned an invalid Wi-Fi profile.".into());
        }
        // SAFETY: successful DPAPI output is LocalAlloc memory of the reported length.
        // Copy it, clear the allocation (including plaintext after decrypt), then free once.
        let result = unsafe {
            let bytes = std::slice::from_raw_parts_mut(output.data, output.length as usize);
            let result = bytes.to_vec();
            for byte in bytes {
                ptr::write_volatile(byte, 0);
            }
            LocalFree(output.data.cast());
            result
        };
        Ok(result)
    }
    pub(super) fn protect(input: &[u8]) -> Result<Vec<u8>, String> {
        transform(input, true)
    }
    pub(super) fn unprotect(input: &[u8]) -> Result<Vec<u8>, String> {
        transform(input, false)
    }
    pub(super) fn replace(source: &Path, destination: &Path) -> Result<(), String> {
        fn wide(path: &Path) -> Result<Vec<u16>, String> {
            let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
            if units.contains(&0) {
                return Err("The Wi-Fi profile destination is invalid.".into());
            }
            units.push(0);
            Ok(units)
        }
        let source = wide(source)?;
        let destination = wide(destination)?;
        // SAFETY: both nul-terminated paths live across the call. Same-directory
        // move replaces an existing file; WRITE_THROUGH=8 avoids a deferred move.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
            Err("Could not replace the protected Wi-Fi profile.".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;
    pub(super) fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("Saved Wi-Fi profiles require Windows DPAPI.".into())
    }
    pub(super) fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("Saved Wi-Fi profiles require Windows DPAPI.".into())
    }
    pub(super) fn replace(_: &Path, _: &Path) -> Result<(), String> {
        Err("Saved Wi-Fi profiles require Windows DPAPI.".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Profile) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "arcane-wifi-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        (
            dir.join("wifi-profile.json"),
            Profile {
                ssid: "Test Network private".into(),
                password: "test-only-secret-93".into(),
                auto_setup: true,
            },
        )
    }
    #[test]
    fn dpapi_roundtrip_overwrite_and_forget() {
        let (path, mut profile) = fixture();
        assert_eq!(load_at(&path).unwrap(), None);
        save_at(&path, &profile).unwrap();
        assert_eq!(load_at(&path).unwrap(), Some(profile.clone()));
        let disk = fs::read_to_string(&path).unwrap();
        assert!(!disk.contains(&profile.password));
        assert!(!disk.contains(&profile.ssid));
        assert!(!format!("{profile:?}").contains(&profile.password));
        let outer: serde_json::Value = serde_json::from_str(&disk).unwrap();
        assert_eq!(outer.as_object().unwrap().len(), 2);
        profile.password = "different-test-only-password".into();
        profile.auto_setup = false;
        save_at(&path, &profile).unwrap();
        assert_eq!(load_at(&path).unwrap(), Some(profile));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        forget_at(&path).unwrap();
        forget_at(&path).unwrap();
        assert_eq!(load_at(&path).unwrap(), None);
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn corrupt_blob_and_validation_are_safe() {
        let (path, mut profile) = fixture();
        save_at(&path, &profile).unwrap();
        let before = fs::read(&path).unwrap();
        profile.ssid = "bad\nnetwork".into();
        assert!(save_at(&path, &profile).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut outer: serde_json::Value = serde_json::from_slice(&before).unwrap();
        let mut blob = STANDARD
            .decode(outer["ciphertext"].as_str().unwrap())
            .unwrap();
        let index = blob.len() / 2;
        blob[index] ^= 0x40;
        outer["ciphertext"] = STANDARD.encode(blob).into();
        fs::write(&path, serde_json::to_vec(&outer).unwrap()).unwrap();
        let error = load_at(&path).unwrap_err();
        assert!(!error.contains(&profile.password));
        fs::write(&path, br#"{"version":1,"ciphertext":"not base64!"}"#).unwrap();
        assert!(load_at(&path).is_err());
        forget_at(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }
}
