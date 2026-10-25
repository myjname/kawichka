//! Data-only profile archives, isolated inspection and transactional restore.
mod archive;
#[cfg(all(test, windows))]
mod full_volume_tests;
mod restore;
mod snapshot;
#[cfg(test)]
mod tests;
#[cfg(test)]
thread_local! { static FAULT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) }; }
#[cfg(test)]
type IoHook = Box<dyn Fn(&str)>;
#[cfg(test)]
thread_local! {static IO_BOUNDARY:std::cell::RefCell<Option<IoHook>>=const{std::cell::RefCell::new(None)};}
fn boundary(_point: &str) {
    #[cfg(test)]
    IO_BOUNDARY.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(_point);
        }
    });
    #[cfg(test)]
    FAULT.with(|at| {
        if at.borrow().as_str() == _point {
            std::process::exit(77);
        }
    });
}
use crate::{Error, Result};
pub use archive::{CaptureSource, ExportOptions, ExportReport, Manifest, inspect};
pub(crate) use restore::recover;
pub use restore::{ApplyReport, RestorePlan, RestoreSummary};
use serde::{Deserialize, Serialize};
pub(crate) use snapshot::Snapshot;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
pub struct Inspected {
    pub(crate) _job: OwnedJob,
    pub(crate) root: PathBuf,
    pub(crate) manifest: Manifest,
    pub(crate) archive_sha256: String,
    pub(crate) snapshot: Snapshot,
}
pub(crate) struct OwnedJob {
    root: PathBuf,
    preserve_if: Option<PathBuf>,
}
impl OwnedJob {
    fn new(root: &Path, operation: &str) -> Result<Self> {
        crate::preview::no_link_chain(root)?;
        if !root.is_absolute() || root.exists() {
            return Err(invalid("new absolute operation folder required"));
        }
        std::fs::create_dir_all(root)?;
        crate::datafs::json(
            &root.join("profile-job.owner"),
            &serde_json::json!({"format":1,"operation":operation}),
        )?;
        Ok(Self {
            root: root.into(),
            preserve_if: None,
        })
    }
}
impl Drop for OwnedJob {
    fn drop(&mut self) {
        if self.preserve_if.as_ref().is_some_and(|p| p.exists()) {
            return;
        }
        if let Some(parent) = self.root.parent() {
            let _ = crate::datafs::remove_owned(&self.root, parent);
        }
    }
}
pub fn cleanup_jobs(root: &Path) -> Result<usize> {
    if root.join("profile-restore.json").exists() {
        return Err(invalid("restore recovery must precede temporary cleanup"));
    }
    let jobs = root.join("jobs");
    crate::preview::no_link_chain(&jobs)?;
    let mut count = 0;
    if !jobs.exists() {
        return Ok(0);
    }
    for item in std::fs::read_dir(&jobs)? {
        let path = item?.path();
        crate::preview::no_link_chain(&path)?;
        let marker = path.join("profile-job.owner");
        crate::preview::no_link_chain(&marker)?;
        if !marker.is_file() {
            continue;
        }
        if marker.metadata()?.len() > 4096 {
            return Err(invalid("unknown profile job owner"));
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(marker)?)?;
        if value["format"] != 1
            || !matches!(
                value["operation"].as_str(),
                Some("export" | "inspect" | "publication")
            )
        {
            return Err(invalid("unknown profile job format; retained"));
        }
        crate::datafs::remove_owned(&path, &jobs)?;
        count += 1;
    }
    Ok(count)
}
impl Inspected {
    pub fn preferences(&self) -> &kavychka_core::preferences::Preferences {
        &self.snapshot.prefs
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn archive_sha256(&self) -> &str {
        &self.archive_sha256
    }
    pub fn validate_files(&self, cancel: &AtomicBool) -> Result<()> {
        for entry in &self.manifest.entries {
            let path = crate::presets::inside(&self.root, &entry.path)?;
            if crate::datafs::hash(&path, cancel)? != (entry.bytes, entry.sha256.clone()) {
                return Err(Error::Invalid(
                    "inspected profile was changed; inspect again".into(),
                ));
            }
        }
        Ok(())
    }
}
fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
fn allowed(path: &str) -> bool {
    if let Some(name) = path.strip_prefix("legacy/profiles/")
        && let Some((id, file)) = name.split_once('/')
    {
        return id.len() == 64
            && id.bytes().all(|b| b.is_ascii_hexdigit())
            && file == "library.sqlite3";
    }
    if path == "library.sqlite3"
        || path == "legacy/library.snapshot.sqlite3"
        || path == "legacy/reminder-state.json"
    {
        return true;
    }
    if let Some(name) = path.strip_prefix("legacy/imported/")
        && let Some((id, file)) = name.split_once('/')
    {
        return id.len() == 64
            && id.bytes().all(|b| b.is_ascii_hexdigit())
            && ["library.snapshot.sqlite3", "reminder-state.json"].contains(&file);
    }
    if let Some(name) = path.strip_prefix("audio/") {
        return !name.contains('/') && media(name);
    }
    if let Some(name) = path.strip_prefix("builtin-days/") {
        return !name.contains('/') && name.ends_with(".json");
    }
    if let Some(name) = path.strip_prefix("presets/") {
        let Some((id, tail)) = name.split_once('/') else {
            return false;
        };
        return !id.is_empty()
            && id.len() <= 128
            && id.chars().all(|c| c.is_ascii_alphanumeric())
            && (tail == "preset.json"
                || media(tail)
                || ["png", "jpg", "jpeg", "gif", "webp"].contains(
                    &Path::new(tail)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                ));
    }
    false
}
fn media(name: &str) -> bool {
    ["wav", "flac", "mp3", "ogg", "opus", "m4a", "aac"].contains(
        &Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
    )
}
