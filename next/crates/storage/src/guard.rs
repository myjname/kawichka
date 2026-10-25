//! Coordinate with the unchanged WinForms instance before capturing its files.
use crate::{Error, Result};

pub struct LegacyGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl LegacyGuard {
    pub fn acquire() -> Result<Self> {
        Self::acquire_named("Local\\KavychkaBreaks-v1")
    }

    fn acquire_named(name: &str) -> Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
                System::Threading::CreateMutexW,
            };
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: name is NUL-terminated and the handle is owned by this guard.
            let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            // GetLastError must be read before any further OS calls.
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(Error::Busy);
            }
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Ok(Self {})
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn named_mutex_rejects_second_owner_and_releases_on_drop() {
        let name = format!("Local\\Kavychka-isolated-guard-test-{}", std::process::id());
        let first = LegacyGuard::acquire_named(&name).unwrap();
        assert!(matches!(
            LegacyGuard::acquire_named(&name),
            Err(Error::Busy)
        ));
        drop(first);
        assert!(LegacyGuard::acquire_named(&name).is_ok());
    }
}

impl Drop for LegacyGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

pub(crate) fn free_space(path: &std::path::Path) -> Result<Option<u64>> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Some(available))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(None)
    }
}

pub(crate) fn atomic_replace(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)?;
        Ok(())
    }
}
