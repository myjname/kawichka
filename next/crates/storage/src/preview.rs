//! Restartable, explicitly requested preview import. Never switches live storage.
//!
//! Copy DB+WAL into a private capture first, then use SQLite's backup API there.
//! Even a read-only SQLite WAL connection may touch SHM read marks: opening only
//! the private capture guarantees no SQLite writes beside the original DB.
use crate::{Error, Result, guard::LegacyGuard};
use kavychka_core::legacy::{SettingsSnapshot, reconcile};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const COPY_DIRS: &[&str] = &["audio", "presets", "builtin-days"];
// Only this designated, already owned generation may live under a legacy root.
// Inventory never descends into it; arbitrary nested preview targets still fail.
fn owned_generation(source: &Path, target: &Path) -> bool {
    let Ok(source) = source.canonicalize() else {
        return false;
    };
    let parent = source.join(".kavychka-next");
    target == parent.join("legacy")
        && std::fs::read(parent.join("shell-sandbox.owner"))
            .is_ok_and(|b| b == b"kavychka-shell-sandbox-v1\n")
}
const COPY_FILES: &[&str] = &["reminder-state.json", "legacy-personalization.json"];
const DB_FILES: &[&str] = &[
    "library.sqlite3",
    "library.sqlite3-wal",
    "library.sqlite3-shm",
];
const KNOWN_TABLES: &[&str] = &[
    "settings",
    "phrases",
    "plans",
    "voice_rotation",
    "preparation",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inventory {
    pub source: PathBuf,
    pub files: Vec<Asset>,
    pub fingerprint: String,
    pub copy_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Capturing,
    Copying,
    Verifying,
    Ready,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    format: u32,
    inventory: Inventory,
    phase: Phase,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub format: u32,
    pub source: PathBuf,
    pub phase: Phase,
    pub copied_bytes: u64,
    pub files: usize,
    pub rows: BTreeMap<String, u64>,
    pub model_roots: BTreeMap<String, String>,
    pub models_copied: bool,
    pub live_storage_switched: bool,
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn not_link(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let mut linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= meta.file_attributes() & 0x400 != 0; // all reparse points, including junctions
    }
    if linked {
        return Err(Error::Invalid(
            "linked source/destination files are not supported".into(),
        ));
    }
    Ok(())
}
pub fn no_link_chain(path: &Path) -> Result<()> {
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(_) => not_link(parent)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn open_source(path: &Path) -> Result<File> {
    not_link(path)?;
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Deny concurrent write/delete while reading. This also rejects an
        // unrelated legacy writer not participating in the named mutex.
        opts.share_mode(1);
    }
    Ok(opts.open(path)?)
}
fn hash(path: &Path, cancel: &AtomicBool) -> Result<(u64, String)> {
    let mut input = open_source(path)?;
    let mut digest = Sha256::new();
    let mut count = 0;
    let mut buffer = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        digest.update(&buffer[..n]);
    }
    Ok((count, format!("{:x}", digest.finalize())))
}
fn walk(root: &Path, path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    not_link(path)?;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            walk(root, &entry?.path(), files)?;
        }
    } else if path.is_file() {
        if !path.starts_with(root) {
            return Err(Error::Invalid("asset escapes source".into()));
        }
        files.push(path.to_path_buf());
    }
    Ok(())
}
pub fn inventory(source: &Path, cancel: &AtomicBool) -> Result<Inventory> {
    no_link_chain(source)?;
    let source = source.canonicalize()?;
    if !source.is_dir() {
        return Err(Error::Invalid("source is not a directory".into()));
    }
    let mut paths = Vec::new();
    for directory in COPY_DIRS {
        let path = source.join(directory);
        if path.try_exists()? {
            walk(&source, &path, &mut paths)?;
        }
    }
    for file in COPY_FILES
        .iter()
        .chain(DB_FILES)
        .chain(["settings.json"].iter())
    {
        let path = source.join(file);
        if path.try_exists()? {
            not_link(&path)?;
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(Error::Invalid("no supported legacy data found".into()));
    }
    let mut files = Vec::new();
    for path in paths {
        let (bytes, sha256) = hash(&path, cancel)?;
        files.push(Asset {
            path: path
                .strip_prefix(&source)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            bytes,
            sha256,
        });
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&files)?));
    let copy_bytes = files.iter().map(|file| file.bytes).sum();
    Ok(Inventory {
        source,
        files,
        fingerprint,
        copy_bytes,
    })
}
fn read_json(path: &Path) -> Result<Value> {
    if fs::metadata(path)?.len() > 8_000_000 {
        return Err(Error::Invalid("settings JSON is too large".into()));
    }
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
    )?)
}
fn write_new_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}
fn journal(root: &Path, value: &Journal) -> Result<()> {
    let next = root.join("preview-journal.next.json");
    if next.exists() {
        not_link(&next)?;
        fs::remove_file(&next)?;
    }
    write_new_json(&next, value)?;
    let path = root.join("preview-journal.json");
    if path.exists() {
        not_link(&path)?;
    }
    crate::guard::atomic_replace(&next, &path)?;
    Ok(())
}
fn copy_asset(source: &Path, destination: &Path, asset: &Asset, cancel: &AtomicBool) -> Result<()> {
    cancelled(cancel)?;
    let output = destination.join(&asset.path);
    no_link_chain(&output)?;
    if output.exists() && hash(&output, cancel)? == (asset.bytes, asset.sha256.clone()) {
        return Ok(());
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::Invalid("invalid destination".into()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.copy-part",
        output.file_name().unwrap().to_string_lossy()
    ));
    if temporary.exists() {
        not_link(&temporary)?;
        fs::remove_file(&temporary)?;
    }
    let mut input = open_source(&source.join(&asset.path))?;
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut buf = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        digest.update(&buf[..n]);
        bytes += n as u64;
    }
    out.sync_all()?;
    drop(out);
    if bytes != asset.bytes || format!("{:x}", digest.finalize()) != asset.sha256 {
        return Err(Error::SourceChanged);
    }
    if output.exists() {
        fs::remove_file(&output)?;
    }
    fs::rename(temporary, output)?;
    Ok(())
}

fn database_settings(db: &Connection) -> Result<BTreeMap<String, Value>> {
    // Store.settings() overlays persisted rows on DEFAULTS. Missing rows are
    // not evidence that an old, non-migrated desktop choice should win.
    let mut result = BTreeMap::from([
        ("text_model".into(), json!("text-gemma4-12b")),
        ("runtime_text_model".into(), json!("text-gemma4-12b")),
        ("tts_model".into(), json!("tts-cosyvoice3")),
        ("swear".into(), json!(false)),
        ("runtime".into(), json!(false)),
        ("auto_daily".into(), json!(true)),
        ("daily_auto_keep".into(), json!(true)),
    ]);
    if !table_exists(db, "settings")? {
        return Ok(result);
    }
    let mut query = db.prepare("SELECT key,value FROM settings ORDER BY key")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        // Unknown keys may contain credentials; do not even parse their values.
        if kavychka_core::legacy::INFERENCE_FIELDS.contains(&key.as_str()) {
            let raw: String = row.get(1)?;
            result.insert(key, serde_json::from_str(&raw)?);
        }
    }
    Ok(result)
}
fn table_exists(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )?)
}
fn validate_database(db: &Connection) -> Result<BTreeMap<String, u64>> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if !(0..=2).contains(&version) {
        return Err(Error::Invalid("unsupported legacy database version".into()));
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(Error::Invalid(
            "legacy SQLite integrity check failed".into(),
        ));
    }
    let mut schema =
        db.prepare("SELECT name,type FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")?;
    for item in schema.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (name, kind) = item?;
        if kind == "trigger"
            || kind == "view"
            || kind == "table" && !KNOWN_TABLES.contains(&name.as_str())
        {
            return Err(Error::Invalid("unsupported legacy database schema".into()));
        }
    }
    let mut counts = BTreeMap::new();
    for table in KNOWN_TABLES {
        if table_exists(db, table)? {
            let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
            counts.insert((*table).into(), count as u64);
        }
    }
    if table_exists(db, "phrases")? {
        let mut query =
            db.prepare("SELECT audio_path FROM phrases WHERE audio_path IS NOT NULL")?;
        for item in query.query_map([], |row| row.get::<_, String>(0))? {
            let name = item?;
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
                return Err(Error::Invalid("audio path escapes its directory".into()));
            }
        }
    }
    Ok(counts)
}

/// Requires a held legacy mutex in production. Tests call the private inner
/// function only for independent temporary roots, never real user storage.
pub fn prepare(
    source: &Path,
    destination: &Path,
    limit: u64,
    cancel: &AtomicBool,
    _guard: &LegacyGuard,
    progress: impl FnMut(Phase),
) -> Result<Report> {
    prepare_steps(source, destination, limit, cancel, progress)
}

fn prepare_steps(
    source: &Path,
    destination: &Path,
    limit: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Phase),
) -> Result<Report> {
    let inventory = inventory(source, cancel)?;
    if inventory.copy_bytes > limit {
        return Err(Error::Invalid(
            "preview exceeds backup limit; originals were not changed".into(),
        ));
    }
    no_link_chain(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| Error::Invalid("destination requires a parent".into()))?
        .canonicalize()?;
    let target = parent.join(
        destination
            .file_name()
            .ok_or_else(|| Error::Invalid("invalid destination".into()))?,
    );
    let source_key = inventory.source.to_string_lossy().to_lowercase();
    let target_key = target.to_string_lossy().to_lowercase();
    if source_key == target_key
        || (target_key.starts_with(&(source_key.clone() + "\\"))
            || target_key.starts_with(&(source_key.clone() + "/")))
            && !owned_generation(source, &target)
        || source_key.starts_with(&(target_key.clone() + "\\"))
        || source_key.starts_with(&(target_key.clone() + "/"))
    {
        return Err(Error::Invalid(
            "preview must be separate from source".into(),
        ));
    }
    // Source bytes include the private capture; allow space for backup/VACUUM
    // plus metadata. Models are excluded from both the inventory and estimate.
    let database_bytes: u64 = inventory
        .files
        .iter()
        .filter(|file| DB_FILES.contains(&file.path.as_str()))
        .map(|file| file.bytes)
        .sum();
    let peak = inventory
        .copy_bytes
        .saturating_add(database_bytes.saturating_mul(2))
        .saturating_add(1_000_000);
    if crate::guard::free_space(&parent)?.is_some_and(|available| available < peak) {
        return Err(Error::Invalid(
            "not enough free space for capture, backup and verification".into(),
        ));
    }
    let fresh = !target.exists() || fs::read_dir(&target)?.next().is_none();
    if !fresh
        && !target.join("preview-journal.json").is_file()
        && !target.join("preview-journal.next.json").is_file()
    {
        return Err(Error::Invalid("destination is not an owned preview".into()));
    }
    fs::create_dir_all(&target)?;
    let lock_path = target.join(".preview.lock");
    if lock_path.exists() {
        not_link(&lock_path)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().map_err(|_| Error::Busy)?;
    let mut state = Journal {
        format: 1,
        inventory: inventory.clone(),
        phase: Phase::Capturing,
    };
    if !fresh {
        let saved = if target.join("preview-journal.json").exists() {
            target.join("preview-journal.json")
        } else {
            target.join("preview-journal.next.json")
        };
        not_link(&saved)?;
        let previous: Journal = serde_json::from_value(read_json(&saved)?)?;
        if previous.format != 1 || previous.inventory != inventory {
            return Err(Error::SourceChanged);
        }
        state = previous;
    } else {
        let mut ignore = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target.join(".gitignore"))?;
        ignore.write_all(b"# Private Kavychka preview data\n*\n")?;
        ignore.sync_all()?;
        journal(&target, &state)?;
    }
    cancelled(cancel)?;
    state.phase = Phase::Capturing;
    journal(&target, &state)?;
    progress(state.phase.clone());
    let old_report = target.join("preview-report.json");
    if old_report.exists() {
        not_link(&old_report)?;
        fs::remove_file(old_report)?;
    }
    // A stable byte capture is isolated from SQLite's sidecar bookkeeping.
    let capture = target.join("capture");
    no_link_chain(&capture)?;
    fs::create_dir_all(&capture)?;
    for file in &inventory.files {
        if DB_FILES.contains(&file.path.as_str()) || file.path == "settings.json" {
            copy_asset(&inventory.source, &capture, file, cancel)?;
        }
    }
    let snapshot_path = target.join("library.snapshot.sqlite3");
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let path = PathBuf::from(format!("{}{suffix}", snapshot_path.display()));
        if path.exists() {
            not_link(&path)?;
            fs::remove_file(path)?;
        }
    }
    let mut snapshot = Connection::open(&snapshot_path)?;
    let (counts, settings) = if capture.join("library.sqlite3").exists() {
        let source_db = Connection::open_with_flags(
            capture.join("library.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        source_db.pragma_update(None, "trusted_schema", false)?;
        let counts = validate_database(&source_db)?;
        let raw = if capture.join("settings.json").exists() {
            read_json(&capture.join("settings.json"))?
        } else {
            json!({})
        };
        let raw = raw
            .as_object()
            .ok_or_else(|| Error::Invalid("desktop settings must be an object".into()))?;
        let settings = reconcile(raw, &database_settings(&source_db)?)?;
        {
            let backup = Backup::new(&source_db, &mut snapshot)?;
            loop {
                cancelled(cancel)?;
                match backup.step(64)? {
                    StepResult::Done => break,
                    StepResult::More => (),
                    _ => return Err(Error::Busy),
                }
            }
        }
        (counts, settings)
    } else {
        let raw = if capture.join("settings.json").exists() {
            read_json(&capture.join("settings.json"))?
        } else {
            json!({})
        };
        let raw = raw
            .as_object()
            .ok_or_else(|| Error::Invalid("desktop settings must be an object".into()))?;
        (BTreeMap::new(), reconcile(raw, &BTreeMap::new())?)
    };
    state.phase = Phase::Copying;
    journal(&target, &state)?;
    progress(state.phase.clone());
    for asset in &inventory.files {
        if !DB_FILES.contains(&asset.path.as_str()) && asset.path != "settings.json" {
            copy_asset(&inventory.source, &target, asset, cancel)?;
        }
    }
    if table_exists(&snapshot, "phrases")? {
        let mut query = snapshot
            .prepare("SELECT audio_path FROM phrases WHERE deleted=0 AND audio_path IS NOT NULL")?;
        for name in query.query_map([], |row| row.get::<_, String>(0))? {
            if !target.join("audio").join(name?).is_file() {
                return Err(Error::Invalid("a live phrase references missing audio; original row was preserved, preview is not ready".into()));
            }
        }
    }
    let model_roots = model_roots(&settings, &inventory.source)?;
    snapshot.pragma_update(None, "secure_delete", true)?;
    snapshot.execute_batch(
        "BEGIN; CREATE TABLE app_settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);",
    )?;
    snapshot.execute(
        "INSERT INTO app_settings VALUES(1,?1)",
        [serde_json::to_string(&settings)?],
    )?;
    if table_exists(&snapshot, "settings")? {
        snapshot.execute("DELETE FROM settings", [])?;
    }
    snapshot.execute_batch("PRAGMA user_version=3; COMMIT; VACUUM;")?;
    state.phase = Phase::Verifying;
    journal(&target, &state)?;
    progress(state.phase.clone());
    let check: String = snapshot.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(Error::Invalid("preview verification failed".into()));
    }
    for (table, count) in &counts {
        if table == "settings" {
            continue;
        }
        let copied: i64 =
            snapshot.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        if copied as u64 != *count {
            return Err(Error::Invalid("row count mismatch".into()));
        }
    }
    if self::inventory(source, cancel)? != inventory {
        return Err(Error::SourceChanged);
    }
    for file in &inventory.files {
        if !DB_FILES.contains(&file.path.as_str())
            && file.path != "settings.json"
            && hash(&target.join(&file.path), cancel)? != (file.bytes, file.sha256.clone())
        {
            return Err(Error::Invalid("copied asset verification failed".into()));
        }
    }
    let report = Report {
        format: 1,
        source: inventory.source.clone(),
        phase: Phase::Ready,
        copied_bytes: inventory.copy_bytes,
        files: inventory.files.len(),
        rows: counts,
        model_roots,
        models_copied: false,
        live_storage_switched: false,
    };
    drop(snapshot);
    let report_path = target.join("preview-report.json");
    if report_path.exists() {
        not_link(&report_path)?;
        fs::remove_file(&report_path)?;
    }
    write_new_json(&report_path, &report)?;
    // Secret-bearing raw captures are never part of a ready generation.
    for name in DB_FILES.iter().chain(["settings.json"].iter()) {
        let path = capture.join(name);
        if path.exists() {
            not_link(&path)?;
            fs::remove_file(path)?;
        }
    }
    let mut ready_files = Vec::new();
    walk(&target, &target, &mut ready_files)?;
    let stored_bytes = ready_files
        .iter()
        .try_fold(0u64, |total, path| -> Result<u64> {
            Ok(total.saturating_add(fs::metadata(path)?.len()))
        })?;
    if stored_bytes.saturating_add(1024) > limit {
        fs::remove_file(&report_path)?;
        return Err(Error::Invalid(
            "verified preview exceeds backup limit including metadata; increase the limit".into(),
        ));
    }
    state.phase = Phase::Ready;
    journal(&target, &state)?;
    Ok(report)
}

fn model_roots(settings: &SettingsSnapshot, source: &Path) -> Result<BTreeMap<String, String>> {
    let mut roots = BTreeMap::new();
    if let Some(paths) = settings
        .inference
        .get("model_paths")
        .and_then(Value::as_object)
    {
        for (id, path) in paths {
            let path = path
                .as_str()
                .ok_or_else(|| Error::Invalid("invalid model path".into()))?;
            roots.insert(id.clone(), path.into());
        }
    }
    let models = source.join("models");
    if models.try_exists()? {
        not_link(&models)?;
        if !models.is_dir() {
            return Err(Error::Invalid("model root is not a directory".into()));
        }
        for entry in fs::read_dir(models)? {
            let entry = entry?;
            not_link(&entry.path())?;
            if entry.file_type()?.is_dir() {
                roots
                    .entry(entry.file_name().to_string_lossy().into())
                    .or_insert_with(|| entry.path().to_string_lossy().into());
            }
        }
    }
    Ok(roots)
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
