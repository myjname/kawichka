//! Preserve the existing shared-location pointer. This module never creates data.
use crate::{Error, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Pointer {
    directory: PathBuf,
}

pub fn resolve(local: &Path) -> Result<PathBuf> {
    let pointer = local.join("Kavychka-storage.json");
    if !pointer.try_exists()? {
        return Ok(local.join("Kavychka"));
    }
    crate::preview::no_link_chain(&pointer)?;
    let bytes = std::fs::read(pointer)?;
    let pointer: Pointer =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
    if !pointer.directory.is_absolute()
        || !pointer.directory.is_dir()
        || !pointer.directory.join("storage-owner.json").is_file()
    {
        return Err(Error::Invalid(
            "selected storage is unavailable; reconnect its disk (no fallback download)".into(),
        ));
    }
    crate::preview::no_link_chain(&pointer.directory)?;
    let owner_path = pointer.directory.join("storage-owner.json");
    crate::preview::no_link_chain(&owner_path)?;
    let owner: serde_json::Value = serde_json::from_slice(&std::fs::read(owner_path)?)?;
    if owner.get("application").and_then(serde_json::Value::as_str) != Some("Kavychka") {
        return Err(Error::Invalid(
            "selected directory is not Kavychka-owned storage".into(),
        ));
    }
    Ok(pointer.directory.canonicalize()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_disk_never_falls_back_or_creates_new_data() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(resolve(root.path()).unwrap(), root.path().join("Kavychka"));
        assert!(!root.path().join("Kavychka").exists());
        std::fs::write(
            root.path().join("Kavychka-storage.json"),
            serde_json::to_vec(&serde_json::json!({"directory":root.path().join("unplugged")}))
                .unwrap(),
        )
        .unwrap();
        assert!(resolve(root.path()).is_err());
        assert!(!root.path().join("Kavychka").exists());
    }
    #[test]
    fn chosen_root_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("Общая папка");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(
            data.join("storage-owner.json"),
            "{\"application\":\"Kavychka\",\"version\":1}",
        )
        .unwrap();
        std::fs::write(
            root.path().join("Kavychka-storage.json"),
            serde_json::to_vec(&serde_json::json!({"directory":data})).unwrap(),
        )
        .unwrap();
        assert_eq!(resolve(root.path()).unwrap(), data.canonicalize().unwrap());
    }

    #[test]
    fn unreadable_format_is_an_error_not_a_default_location() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Kavychka-storage.json"), b"invalid JSON").unwrap();
        assert!(resolve(root.path()).is_err());
        assert!(!root.path().join("Kavychka").exists());
    }
}
