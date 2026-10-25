//! Bounded, cancellable I/O shared by explicit profile/storage operations.
use crate::{Error, Result, preview::no_link_chain};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
pub(crate) fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) fn id() -> String {
    format!(
        "p{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
pub(crate) fn hash(path: &Path, cancel: &AtomicBool) -> Result<(u64, String)> {
    no_link_chain(path)?;
    let mut file = File::open(path)?;
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        check(cancel)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or_else(|| Error::Invalid("file size overflow".into()))?;
        sha.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", sha.finalize())))
}
pub(crate) fn copy(from: &Path, to: &Path, cancel: &AtomicBool) -> Result<(u64, String)> {
    no_link_chain(from)?;
    no_link_chain(to)?;
    fs::create_dir_all(
        to.parent()
            .ok_or_else(|| Error::Invalid("missing parent".into()))?,
    )?;
    let mut source = File::open(from)?;
    let mut target = OpenOptions::new().create_new(true).write(true).open(to)?;
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        check(cancel)?;
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        target.write_all(&buffer[..n])?;
        size = size
            .checked_add(n as u64)
            .ok_or_else(|| Error::Invalid("file size overflow".into()))?;
        sha.update(&buffer[..n]);
    }
    target.sync_all()?;
    Ok((size, format!("{:x}", sha.finalize())))
}
pub(crate) fn json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    no_link_chain(path)?;
    let tmp = path.with_file_name(format!("{}.part", id()));
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    crate::guard::atomic_replace(&tmp, path)
}
pub(crate) fn publish(from: &Path, to: &Path) -> Result<()> {
    no_link_chain(from)?;
    no_link_chain(to)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                a.as_ptr(),
                b.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(from, to)?;
        fs::remove_file(from)?;
    }
    Ok(())
}
pub(crate) fn tree(root: &Path, cancel: &AtomicBool) -> Result<Vec<String>> {
    fn walk(root: &Path, path: &Path, out: &mut Vec<String>, cancel: &AtomicBool) -> Result<()> {
        check(cancel)?;
        no_link_chain(path)?;
        for item in fs::read_dir(path)? {
            let item = item?;
            let path = item.path();
            no_link_chain(&path)?;
            if item.file_type()?.is_dir() {
                walk(root, &path, out, cancel)?;
            } else if item.file_type()?.is_file() {
                if out.len() >= 20000 {
                    return Err(Error::Invalid("profile exceeds 20,000 files".into()));
                }
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| Error::Invalid("file outside root".into()))?
                    .to_str()
                    .ok_or_else(|| Error::Invalid("non-Unicode profile filename".into()))?
                    .replace('\\', "/");
                crate::presets::inside(root, &name)?;
                out.push(name);
            } else {
                return Err(Error::Invalid("unsupported filesystem item".into()));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    if root.try_exists()? {
        walk(root, root, &mut out, cancel)?;
    }
    out.sort();
    let mut folded = std::collections::BTreeSet::new();
    for name in &out {
        if !folded.insert(name.to_lowercase()) {
            return Err(Error::Invalid("case-colliding profile paths".into()));
        }
    }
    Ok(out)
}
pub(crate) fn room(path: &Path, needed: u64) -> Result<()> {
    let parent = path
        .ancestors()
        .find(|p| p.is_dir())
        .ok_or_else(|| Error::Invalid("storage disk unavailable".into()))?;
    if crate::guard::free_space(parent)?.is_some_and(|free| free < needed) {
        return Err(Error::Invalid(
            "not enough free disk space; current data retained".into(),
        ));
    }
    Ok(())
}
pub(crate) fn remove_owned(path: &Path, parent: &Path) -> Result<()> {
    if !path.try_exists()? {
        return Ok(());
    }
    no_link_chain(path)?;
    no_link_chain(parent)?;
    let target = path.canonicalize()?;
    let owner = parent.canonicalize()?;
    if target == owner || !target.starts_with(&owner) {
        return Err(Error::Invalid("cleanup target is outside its owner".into()));
    }
    fs::remove_dir_all(target)?;
    Ok(())
}
