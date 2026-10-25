//! Offline, journalled transfer of an installed common root, including weights.
//! No source data is removed. Before the durable commit both pointers roll back;
//! afterwards recovery finishes fencing the old generation and uses the new root.
use crate::{
    Result, atomic, clean_path, digest,
    install::{self, Current, Locator},
};
use kavychka_storage::{
    preview::no_link_chain,
    sandbox::Sandbox,
    transfer::{self, Fence},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const JOURNAL: &str = "storage-move.json";
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Estimate {
    pub source: PathBuf,
    pub target: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub peak_bytes: u64,
    pub breakdown: std::collections::BTreeMap<String, u64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    operation: String,
    state: String,
    previous: Current,
    candidate: Current,
    local: PathBuf,
    locator: Locator,
    update: Option<install::Journal>,
    paths: Vec<(PathBuf, PathBuf)>,
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("Storage transfer cancelled; source retained".into())
    } else {
        Ok(())
    }
}
fn inactive_update(root: &Path) -> Result<()> {
    let path = clean_path(root, "transaction.json")?;
    if path.exists() {
        let j: install::Journal = serde_json::from_slice(&fs::read(path)?)?;
        if j.format != 1 || !["complete", "rolled_back"].contains(&j.state.as_str()) {
            return Err("Finish installation/update recovery before moving storage".into());
        }
    }
    Ok(())
}
fn target_path(source: &Path, target: &Path, program: &Path) -> Result<()> {
    if !target.is_absolute()
        || target
            .to_string_lossy()
            .split(['\\', '/'])
            .any(|s| s == ".." || s == ".")
        || target.parent().is_none()
        || target.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("Absolute non-root destination without dot components required".into());
    }
    no_link_chain(target)?;
    // Resolve the nearest existing ancestor before the containment check (.. and
    // case aliases must not turn an apparently new path into part of the source).
    let mut tail = vec![];
    let mut at = target;
    while !at.try_exists()? {
        tail.push(
            at.file_name()
                .ok_or("destination drive missing")?
                .to_owned(),
        );
        at = at.parent().ok_or("destination drive missing")?;
    }
    let mut resolved = at.canonicalize()?;
    for part in tail.into_iter().rev() {
        resolved.push(part);
    }
    for owned in [source.canonicalize()?, program.canonicalize()?] {
        if resolved.starts_with(&owned) || owned.starts_with(&resolved) {
            return Err("Destination must be separate from current data and programs".into());
        }
    }
    if target.try_exists()? {
        return Err("Choose a new destination folder; nothing there will be overwritten".into());
    }
    Ok(())
}
fn skip(name: &str) -> bool {
    name == ".kavychka-next/shell.lock"
        || name == ".kavychka-next/storage-transfer.json"
        || name == ".kavychka-next/shell.sqlite3"
        || name.starts_with(".kavychka-next/shell.sqlite3-")
        || name.starts_with(".kavychka-next/jobs/")
}
fn inventory(common: &Path, cancel: &AtomicBool) -> Result<Vec<(String, u64)>> {
    fn walk(
        base: &Path,
        at: &Path,
        out: &mut Vec<(String, u64)>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        cancelled(cancel)?;
        no_link_chain(at)?;
        for entry in fs::read_dir(at)? {
            cancelled(cancel)?;
            let entry = entry?;
            let path = entry.path();
            no_link_chain(&path)?;
            let name = path
                .strip_prefix(base)?
                .to_str()
                .ok_or("non-Unicode storage path")?
                .replace('\\', "/");
            clean_path(base, &name)?;
            if skip(&name) || name == ".kavychka-next/jobs" {
                continue;
            }
            if entry.file_type()?.is_dir() {
                walk(base, &path, out, cancel)?;
            } else if entry.file_type()?.is_file() {
                if out.len() >= 100000 {
                    return Err("Storage exceeds 100,000 files".into());
                }
                out.push((name, entry.metadata()?.len()));
            } else {
                return Err("Unsupported storage item".into());
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(common, common, &mut files, cancel)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names = std::collections::BTreeSet::new();
    if files.iter().any(|(n, _)| !names.insert(n.to_lowercase())) {
        return Err("Case-colliding storage files".into());
    }
    Ok(files)
}
pub fn estimate(root: &Path, target: &Path, cancel: &AtomicBool) -> Result<Estimate> {
    estimate_with_models(root, target, cancel, &Default::default())
}
pub fn estimate_with_models(
    root: &Path,
    target: &Path,
    cancel: &AtomicBool,
    models: &std::collections::BTreeMap<String, String>,
) -> Result<Estimate> {
    inactive_update(root)?;
    let current = install::current(root)?.ok_or("Installed data pointer missing")?;
    target_path(&current.common, target, root)?;
    if transfer::fence(&current.data)?.is_some() {
        return Err("Recover the previous storage transfer first".into());
    }
    let files = inventory(&current.common, cancel)?;
    let mut bytes = fs::metadata(current.data.join("shell.sqlite3"))?.len();
    let mut breakdown = std::collections::BTreeMap::<String, u64>::new();
    breakdown.insert("data".into(), bytes);
    let owned_models: Vec<_> = models
        .values()
        .filter_map(|p| transfer::owned_tail(Path::new(p), &current.common))
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        .collect();
    for (name, size) in &files {
        bytes = bytes.checked_add(*size).ok_or("storage size overflow")?;
        let lower = name.to_lowercase();
        let local = lower.strip_prefix(".kavychka-next/").unwrap_or(&lower);
        let category = if owned_models
            .iter()
            .any(|p| lower == *p || lower.starts_with(&format!("{p}/")))
            || local.starts_with("models/")
            || local.starts_with("components/models/")
        {
            "models"
        } else if local.starts_with("runtime/")
            || local.starts_with("llama/")
            || local.starts_with("vendor/")
            || local.starts_with("components/")
        {
            "runtimes"
        } else if local.starts_with("audio/") {
            "audio"
        } else if local.starts_with("presets/") {
            "presets"
        } else if local.starts_with("cache/") {
            "cache"
        } else if local.starts_with("downloads/") {
            "downloads"
        } else if local.starts_with("recovery/") {
            "recovery"
        } else if local.starts_with("quarantine/") {
            "quarantine"
        } else if local.starts_with("exports/") {
            "exports"
        } else if local.starts_with("logs/") {
            "logs"
        } else {
            "data"
        };
        let entry = breakdown.entry(category.into()).or_default();
        *entry = entry.checked_add(*size).ok_or("storage size overflow")?;
    }
    // Database + WAL and manifest/journal headroom; originals stay on their disk.
    let wal = current.data.join("shell.sqlite3-wal");
    if wal.exists() {
        bytes = bytes
            .checked_add(wal.metadata()?.len())
            .ok_or("storage size overflow")?;
        *breakdown.get_mut("data").unwrap() += wal.metadata()?.len();
    }
    let peak = bytes
        .checked_add(64_000_000)
        .ok_or("storage size overflow")?;
    crate::windows::disk_space(
        target
            .ancestors()
            .find(|p| p.is_dir())
            .ok_or("destination disk unavailable")?,
        peak,
    )?;
    Ok(Estimate {
        source: current.common,
        target: target.to_owned(),
        files: files.len() + 1,
        bytes,
        peak_bytes: peak,
        breakdown,
    })
}
fn read(root: &Path) -> Result<Option<Journal>> {
    let path = clean_path(root, JOURNAL)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    if path.metadata()?.len() > 32000 {
        return Err("storage journal too large".into());
    }
    let j: Journal = serde_json::from_slice(&fs::read(path)?)?;
    crate::package::version(&j.previous.version)?;
    if j.format != 1
        || ![
            "copying",
            "switching",
            "complete",
            "finished",
            "rolled_back",
        ]
        .contains(&j.state.as_str())
        || j.previous.format != 1
        || j.candidate.format != 1
        || j.previous.version != j.candidate.version
        || j.previous.data != j.previous.common.join(".kavychka-next")
        || j.candidate.data != j.candidate.common.join(".kavychka-next")
        || !j.local.is_absolute()
        || j.locator.format != 1
        || j.locator.program_root != root
        || j.locator.common_root != j.previous.common
        || !j.previous.common.is_absolute()
        || !j.candidate.common.is_absolute()
    {
        return Err("invalid storage journal".into());
    }
    Fence {
        format: 1,
        operation: j.operation.clone(),
        state: "pending".into(),
        source: j.previous.data.clone(),
        target: j.candidate.data.clone(),
    }
    .validate()?;
    no_link_chain(&j.local)?;
    validate_paths(&j.paths)?;
    Ok(Some(j))
}
fn validate_paths(paths: &[(PathBuf, PathBuf)]) -> Result<()> {
    if paths.len() > 16
        || paths
            .iter()
            .any(|(a, b)| !a.is_absolute() || !b.is_absolute() || a == b)
    {
        return Err("Invalid storage path history".into());
    }
    Ok(())
}
fn paths(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let file = clean_path(root, "storage-paths.json")?;
    if !file.exists() {
        return Ok(vec![]);
    }
    if file.metadata()?.len() > 16000 {
        return Err("Storage path history too large".into());
    }
    let paths: Vec<(PathBuf, PathBuf)> = serde_json::from_slice(&fs::read(file)?)?;
    validate_paths(&paths)?;
    Ok(paths)
}
fn write_update(root: &Path, j: &Journal, moved: bool) -> Result<()> {
    if let Some(update) = &j.update {
        let mut update = update.clone();
        if moved {
            let change = |c: &mut Current| {
                if c.common == j.previous.common {
                    if let Some(path) = &c.legacy
                        && let Some(tail) = transfer::owned_tail(path, &j.previous.common)
                    {
                        c.legacy = Some(j.candidate.common.join(tail));
                    }
                    c.common = j.candidate.common.clone();
                    c.data = j.candidate.data.clone();
                }
            };
            change(&mut update.candidate);
            if let Some(c) = &mut update.previous {
                change(c);
            }
        }
        atomic(&root.join("transaction.json"), &update)?;
    }
    let mut history = j.paths.clone();
    if moved {
        history.push((j.previous.common.clone(), j.candidate.common.clone()));
    }
    atomic(&root.join("storage-paths.json"), &history)
}
/// Old-version recovery archives retain their original bytes. Rebase their known
/// operational paths only in the restored working copy, through all later moves.
pub fn rebase_restored(root: &Path, data: &Path) -> Result<()> {
    let history = paths(root)?;
    if !history.is_empty() {
        let mut store = Sandbox::open(data)?;
        for (old, new) in history {
            store.rebase_storage(&old, &new)?;
        }
    }
    Ok(())
}
fn fence(j: &Journal, state: &str) -> Fence {
    Fence {
        format: 1,
        operation: j.operation.clone(),
        state: state.into(),
        source: j.previous.data.clone(),
        target: j.candidate.data.clone(),
    }
}
fn remove_fence(data: &Path, operation: &str) -> Result<()> {
    if let Some(f) = transfer::fence(data)? {
        if f.operation != operation {
            return Err("Storage fence belongs to another transfer".into());
        }
        fs::remove_file(data.join("storage-transfer.json"))?;
    }
    Ok(())
}
/// Called with the installation lock, before update recovery or launcher reads.
pub fn recover(root: &Path) -> Result<()> {
    recover_with(root, |_| Ok(()))
}
fn recover_with(root: &Path, mut boundary: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let Some(mut j) = read(root)? else {
        return Ok(());
    };
    if j.state == "rolled_back" {
        return Ok(());
    }
    if j.state == "finished" {
        if j.previous.data.is_dir()
            && transfer::fence(&j.previous.data)?
                .is_some_and(|f| f.operation == j.operation && f.state == "pending")
        {
            fence(&j, "retired").write(&j.previous.data)?;
        }
        return Ok(());
    }
    if j.state == "complete" {
        if !j.candidate.data.is_dir() {
            return Err("Relocated data disk unavailable; no fallback library created".into());
        }
        // The commit already names the new root. Do not undo new work there.
        atomic(&root.join("current.json"), &j.candidate)?;
        boundary("recover_current")?;
        install::register_locator(&j.local, root, &j.candidate)?;
        boundary("recover_locator")?;
        write_update(root, &j, true)?;
        remove_fence(&j.candidate.data, &j.operation)?;
        if j.previous.data.is_dir() {
            fence(&j, "retired").write(&j.previous.data)?;
        }
        boundary("recover_fence")?;
        j.state = "finished".into();
        atomic(&root.join(JOURNAL), &j)?;
    } else {
        // New files are deliberately retained as a fenced, inactive candidate.
        atomic(&root.join("current.json"), &j.previous)?;
        boundary("recover_current")?;
        atomic(&j.local.join("Kavychka-next-install.json"), &j.locator)?;
        boundary("recover_locator")?;
        write_update(root, &j, false)?;
        remove_fence(&j.previous.data, &j.operation)?;
        boundary("recover_fence")?;
        j.state = "rolled_back".into();
        atomic(&root.join(JOURNAL), &j)?;
    }
    Ok(())
}
fn copy_file(from: &Path, to: &Path, cancel: &AtomicBool) -> Result<(u64, String, fs::File)> {
    no_link_chain(from)?;
    no_link_chain(to)?;
    // Deny concurrent Windows writers/deleters throughout the copy and source hash.
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut input = options.open(from)?;
    fs::create_dir_all(to.parent().ok_or("missing target parent")?)?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(to)?;
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
        size = size.checked_add(n as u64).ok_or("size overflow")?;
    }
    output.sync_all()?;
    drop(output);
    let sha = format!("{:x}", hash.finalize());
    if checked_hash(to, cancel)? != sha || checked_hash(from, cancel)? != sha {
        return Err("Storage changed during copy; source retained".into());
    }
    Ok((size, sha, input))
}
fn checked_hash(path: &Path, cancel: &AtomicBool) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut input = fs::File::open(path)?;
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn apply(
    root: &Path,
    local: &Path,
    target: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(&str),
) -> Result<Current> {
    apply_with(root, local, target, cancel, progress, |_| Ok(()))
}
fn apply_with(
    root: &Path,
    local: &Path,
    target: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&str),
    mut boundary: impl FnMut(&str) -> Result<()>,
) -> Result<Current> {
    let _guard = install::owned(root)?;
    recover(root)?;
    inactive_update(root)?;
    let previous = install::current(root)?.ok_or("Installed data missing")?;
    let locator = install::locate(local)?.ok_or("Installed locator missing")?;
    if locator.program_root != root || locator.common_root != previous.common {
        return Err("Locator and installed data disagree".into());
    }
    let _legacy = if previous.common.join("library.sqlite3").exists()
        || previous.common.join("settings.json").exists()
    {
        Some(kavychka_storage::guard::LegacyGuard::acquire()?)
    } else {
        None
    };
    let source = Sandbox::open(&previous.data)?; // Fails for a running app, before any writes.
    source.freeze_for_transfer()?;
    let estimate =
        estimate_with_models(root, target, cancel, &source.load()?.generation.model_paths)?;
    let files = inventory(&previous.common, cancel)?;
    let operation = format!(
        "m{:x}{:x}",
        std::process::id(),
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .ok_or("clock out of range")?
    );
    let mut candidate = previous.clone();
    candidate.common = target.to_owned();
    candidate.data = target.join(".kavychka-next");
    if let Some(path) = &candidate.legacy
        && let Some(tail) = transfer::owned_tail(path, &previous.common)
    {
        candidate.legacy = Some(target.join(tail));
    }
    // Rust bootstrap hints follow the copied Origin. Saved WinForms startup
    // integration and the original common files are not changed.
    let update_file = clean_path(root, "transaction.json")?;
    let update = if update_file.exists() {
        Some(serde_json::from_slice(&fs::read(update_file)?)?)
    } else {
        None
    };
    let paths = paths(root)?;
    if paths.len() >= 16 {
        return Err("Storage move history limit reached; preserve a fresh profile first".into());
    }
    let mut j = Journal {
        format: 1,
        operation,
        state: "copying".into(),
        previous,
        candidate,
        local: local.to_owned(),
        locator,
        update,
        paths,
    };
    atomic(&root.join(JOURNAL), &j)?;
    let run = (|| -> Result<Current> {
        fence(&j, "pending").write(&j.previous.data)?;
        boundary("journal")?;
        cancelled(cancel)?;
        fs::create_dir_all(&j.candidate.data)?;
        fence(&j, "pending").write(&j.candidate.data)?;
        let mut copied = vec![];
        let mut source_locks = vec![];
        for (name, expected) in files {
            progress(&name);
            let (bytes, sha, lock) = copy_file(
                &clean_path(&j.previous.common, &name)?,
                &clean_path(&j.candidate.common, &name)?,
                cancel,
            )?;
            source_locks.push(lock);
            if bytes != expected {
                return Err("Storage size changed during transfer".into());
            }
            copied.push(serde_json::json!({"path":name,"bytes":bytes,"sha256":sha}));
        }
        source.backup_database(&j.candidate.data.join("shell.sqlite3"))?;
        boundary("files")?;
        cancelled(cancel)?;
        {
            let mut probe = Sandbox::open_transfer(&j.candidate.data, &j.operation)?;
            probe.rebase_storage(&j.previous.common, &j.candidate.common)?;
            let prefs = probe.load()?;
            prefs
                .validate()
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            probe.load_state()?;
            probe.phrases()?;
            if probe.origin()?.is_some_and(|o| {
                !o.snapshot.is_dir() || !o.snapshot.join("library.snapshot.sqlite3").is_file()
            }) {
                return Err("Copied legacy snapshot missing".into());
            }
        }
        // Every non-database file was verified while copying. The authoritative DB
        // is the SQLite Backup API copy, checked again through the working reader.
        atomic(
            &j.candidate.data.join("storage-transfer.manifest.json"),
            &serde_json::json!({"format":1,"operation":j.operation,"estimate":estimate,"files":copied,"database_sha256":digest(&j.candidate.data.join("shell.sqlite3"))?}),
        )?;
        boundary("probe")?;
        cancelled(cancel)?;
        j.state = "switching".into();
        atomic(&root.join(JOURNAL), &j)?;
        atomic(&root.join("current.json"), &j.candidate)?;
        boundary("pointer")?;
        install::register_locator(local, root, &j.candidate)?;
        write_update(root, &j, true)?;
        boundary("locator")?;
        j.state = "complete".into();
        atomic(&root.join(JOURNAL), &j)?;
        boundary("commit")?;
        recover(root)?;
        Ok(j.candidate.clone())
    })();
    drop(source);
    if run.is_err() {
        recover(root)?;
    }
    run
}

#[cfg(test)]
mod tests;
