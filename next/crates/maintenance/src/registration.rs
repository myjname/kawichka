//! Per-user Installed apps entry. Portable mode never calls this module.
use crate::Result;
use std::path::Path;
const ENTRY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\KavychkaNext";
pub fn register(root: &Path, version: &str) -> Result<()> {
    change(ENTRY, root, Some(version))
}
pub fn remove(root: &Path) -> Result<()> {
    change(ENTRY, root, None)
}
#[cfg(not(windows))]
fn change(_: &str, _: &Path, _: Option<&str>) -> Result<()> {
    Err("Windows registration unavailable".into())
}
#[cfg(windows)]
fn change(entry: &str, root: &Path, version: Option<&str>) -> Result<()> {
    use windows_sys::Win32::System::Registry::*;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn check(code: u32) -> Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32).into())
        }
    }
    unsafe {
        if !root.is_absolute() || root.to_string_lossy().contains(['"', '\n', '\r']) {
            return Err("Invalid installation registration path".into());
        }
        if let Some(v) = version {
            crate::package::version(v)?;
        }
        let name = wide(entry);
        let mut handle = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            name.as_ptr(),
            0,
            KEY_READ | KEY_WRITE,
            &mut handle,
        );
        if status == 0 {
            let key = Key(handle);
            let mut bytes = vec![0u8; 32768];
            let mut len = bytes.len() as u32;
            let mut kind = 0;
            check(RegQueryValueExW(
                key.0,
                wide("KavychkaOwner").as_ptr(),
                std::ptr::null(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut len,
            ))?;
            if kind != REG_SZ || len < 2 || !len.is_multiple_of(2) {
                return Err("Foreign Installed apps entry retained".into());
            }
            let value = String::from_utf16(
                &bytes[..len as usize]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )?;
            if value.trim_end_matches('\0') != root.to_string_lossy() {
                return Err("Installed apps entry belongs to another installation".into());
            }
        } else if status == 2 {
            if version.is_none() {
                return Ok(());
            }
        } else {
            check(status)?;
        }
        if let Some(v) = version {
            check(RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_WRITE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            ))?;
            let key = Key(handle);
            for (n, s) in [
                ("KavychkaOwner", root.display().to_string()),
                ("DisplayName", "Кавычка / Kavychka".into()),
                ("DisplayVersion", v.into()),
                ("InstallLocation", root.display().to_string()),
                (
                    "DisplayIcon",
                    root.join("Kavychka.exe").display().to_string(),
                ),
                (
                    "UninstallString",
                    format!("\"{}\" --uninstall", root.join("Kavychka.exe").display()),
                ),
            ] {
                let value = wide(&s);
                check(RegSetValueExW(
                    key.0,
                    wide(n).as_ptr(),
                    0,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                ))?;
            }
            for n in ["NoModify", "NoRepair"] {
                check(RegSetValueExW(
                    key.0,
                    wide(n).as_ptr(),
                    0,
                    REG_DWORD,
                    1u32.to_le_bytes().as_ptr(),
                    4,
                ))?;
            }
        } else {
            check(RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()))?;
        }
    }
    Ok(())
}
#[cfg(all(test, windows))]
mod tests {
    #[test]
    #[ignore = "explicit isolated HKCU integration test"]
    fn isolated_registry_owner_and_removal() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let key = format!(
            r"Software\KavychkaTests\Uninstall-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );
        super::change(&key, root, Some("0.1.0")).unwrap();
        assert!(super::change(&key, &root.join("foreign"), None).is_err());
        super::change(&key, root, Some("0.1.1")).unwrap();
        super::change(&key, root, None).unwrap();
        super::change(&key, root, None).unwrap();
    }
}
