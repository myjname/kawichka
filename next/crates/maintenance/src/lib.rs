//! Versioned installation, data-preserving recovery, and authenticated release origin.
pub mod diagnostics;
pub mod install;
pub mod launch_context;
pub mod package;
pub mod portable;
pub mod registration;
pub mod relocate;
pub mod removal;
#[cfg(test)]
mod tests;
pub mod updates;
pub mod windows;
use std::{io::Write, path::Path};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    kavychka_storage::preview::no_link_chain(path)?;
    let temp = path.with_extension("next");
    kavychka_storage::preview::no_link_chain(&temp)?;
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(value)?)?;
        f.sync_all()?;
    }
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        use windows_sys::Win32::Storage::FileSystem::*;
        if MoveFileExW(
            a.as_ptr(),
            b.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(temp, path)?;
    Ok(())
}
pub fn digest(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut file, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
pub fn clean_path(root: &Path, name: &str) -> Result<std::path::PathBuf> {
    if name.is_empty()
        || name.contains(['\\', ':'])
        || name.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || s.ends_with(['.', ' '])
                || s.chars().any(char::is_control)
                || kavychka_storage::presets::is_reserved_component(s)
        })
        || Path::new(name).is_absolute()
    {
        return Err("invalid package path".into());
    }
    let p = root.join(name);
    kavychka_storage::preview::no_link_chain(&p)?;
    Ok(p)
}
pub fn delete_owned_tree(path: &Path, base: &Path) -> Result<()> {
    // Check the supplied paths before canonicalize erases junction/reparse
    // components and could turn cleanup of a stage into deletion of its target.
    kavychka_storage::preview::no_link_chain(path)?;
    kavychka_storage::preview::no_link_chain(base)?;
    let path = path.canonicalize()?;
    let base = base.canonicalize()?;
    if path == base || !path.starts_with(&base) {
        return Err("cleanup escaped owned root".into());
    }
    kavychka_storage::preview::no_link_chain(&path)?;
    fn validate(p: &Path) -> Result<()> {
        for f in std::fs::read_dir(p)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            if f.file_type()?.is_dir() {
                validate(&f.path())?
            }
        }
        Ok(())
    }
    validate(&path)?;
    std::fs::remove_dir_all(path)?;
    Ok(())
}
