//! AppData redirection can be inherited even without a package identity.
//! Check an auto-deleted file before touching persistent installation data.
use crate::Result;
use std::path::{Path, PathBuf};

const HELP: &str = "Установка или запуск из пакетированного приложения (например, Codex) может перенаправить данные в его кеш. Закрой это окно и открой Kavychka.exe или установщик из обычного Проводника Windows. Данные не изменены. / A packaged host may virtualize application data. Open Kavychka or its setup from Windows File Explorer. Data was not changed.";

pub fn packaged() -> Result<bool> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::*, Storage::Packaging::Appx::GetCurrentPackageFullName,
        };
        let mut length = 0;
        match unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) } {
            APPMODEL_ERROR_NO_PACKAGE => Ok(false),
            ERROR_INSUFFICIENT_BUFFER | ERROR_SUCCESS => Ok(true),
            code => Err(std::io::Error::from_raw_os_error(code as i32).into()),
        }
    }
    #[cfg(not(windows))]
    Ok(false)
}

/// GUI setup also changes per-user integration, regardless of selected folders.
pub fn require_unpacked() -> Result<()> {
    if packaged()? {
        return Err(HELP.into());
    }
    let roots = data_roots()?;
    ensure_paths(&roots.iter().map(PathBuf::as_path).collect::<Vec<_>>())
}

fn data_roots() -> Result<Vec<PathBuf>> {
    let mut roots: Vec<PathBuf> = ["LOCALAPPDATA", "APPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(profile).join("AppData"));
    }
    if roots.is_empty() {
        return Err(HELP.into());
    }
    Ok(roots)
}

pub fn ensure_paths(paths: &[&Path]) -> Result<()> {
    #[cfg(windows)]
    {
        let roots = data_roots()?;
        for path in paths {
            if in_package_cache(path)? {
                return Err(HELP.into());
            }
            if in_appdata(path, &roots)? {
                // MSIX may let a new file in an already-real subdirectory
                // through, while virtualizing a new sibling directory. Probe
                // the AppData roots too, not just the nearest target ancestor.
                for root in roots.iter().filter(|root| root.is_dir()) {
                    verify_physical_write(root)?;
                }
                let parent = path.ancestors().find(|p| p.is_dir()).ok_or(HELP)?;
                verify_physical_write(parent)?;
            }
        }
    }
    #[cfg(not(windows))]
    let _ = paths;
    Ok(())
}

fn in_package_cache(path: &Path) -> Result<bool> {
    let parts = folded(path)?;
    Ok(parts
        .windows(4)
        .any(|p| p[0] == "local" && p[1] == "packages" && p[3] == "localcache"))
}

fn folded(path: &Path) -> Result<Vec<String>> {
    let absolute = std::path::absolute(path)?;
    let text = absolute.to_string_lossy().replace('/', "\\");
    let text = if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned()
    };
    let mut parts = Vec::new();
    for part in text.split('\\').filter(|s| !s.is_empty() && *s != ".") {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part.to_lowercase());
        }
    }
    Ok(parts)
}

fn in_appdata(path: &Path, roots: &[PathBuf]) -> Result<bool> {
    let roots: Vec<_> = roots.iter().map(|p| folded(p)).collect::<Result<_>>()?;
    let candidate = folded(path)?;
    if roots.iter().any(|root| candidate.starts_with(root)) {
        return Ok(true);
    }
    if let Some(existing) = path.ancestors().find(|p| p.is_dir()) {
        let physical = folded(&existing.canonicalize()?)?;
        if roots.iter().any(|root| physical.starts_with(root)) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(windows)]
fn verify_physical_write(parent: &Path) -> Result<()> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows_sys::Win32::Storage::FileSystem::*;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = parent.join(format!(
        ".kavychka-context-{}-{stamp}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(0x80000000 | 0x40000000 | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE | FILE_ATTRIBUTE_TEMPORARY)
        .open(&path)?;
    let mut buffer = vec![0u16; 32768];
    let size = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if size == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if size as usize >= buffer.len() {
        return Err("Physical installation path exceeds verification bound".into());
    }
    let actual = PathBuf::from(String::from_utf16(&buffer[..size as usize])?);
    if folded(&actual)? != folded(&path)? {
        return Err(HELP.into());
    }
    // Closing the handle removes the probe, including on every error return.
    drop(file);
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn appdata_matching_handles_case_verbatim_and_component_boundaries() {
        let roots = vec![PathBuf::from(r"C:\Users\Example\AppData")];
        for path in [
            r"C:\Users\Example\AppData\Local\Kavychka",
            r"\\?\C:\USERS\Example\APPDATA\Local\Kavychka",
            r"C:\Users\Example\AppData\Roaming\Programs",
        ] {
            assert!(in_appdata(Path::new(path), &roots).unwrap());
        }
        assert!(!in_appdata(Path::new(r"E:\Kavychka-fixture\data"), &roots).unwrap());
        assert!(!in_appdata(Path::new(r"C:\Users\Example\AppDataElse\data"), &roots).unwrap());
        let unc = vec![PathBuf::from(r"\\server\profile\AppData")];
        assert!(
            in_appdata(
                Path::new(r"\\?\UNC\server\profile\AppData\Local\Kavychka"),
                &unc
            )
            .unwrap()
        );
    }
    #[test]
    fn physical_probe_leaves_no_files_or_directories() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/context-probe-tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        verify_physical_write(dir.path()).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn explicit_package_cache_is_not_persistent_profile_storage() {
        assert!(in_package_cache(Path::new(r"C:\Users\Example\AppData\Local\Packages\OpenAI.Codex_example\LocalCache\Local\Kavychka")).unwrap());
        assert!(!in_package_cache(Path::new(r"C:\Users\Example\AppData\Local\Kavychka")).unwrap());
    }
}
