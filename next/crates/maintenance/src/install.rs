use crate::{Result, atomic, clean_path, package};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Current {
    pub format: u32,
    pub version: String,
    pub common: PathBuf,
    pub data: PathBuf,
    #[serde(default)]
    pub legacy: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub format: u32,
    pub program_root: PathBuf,
    pub common_root: PathBuf,
    pub legacy: Option<PathBuf>,
}
pub fn locate(local: &Path) -> Result<Option<Locator>> {
    let file = local.join("Kavychka-next-install.json");
    kavychka_storage::preview::no_link_chain(&file)?;
    if !file.exists() {
        return Ok(None);
    }
    if file.metadata()?.len() > 16000 {
        return Err("installation locator is too large".into());
    }
    let l: Locator = serde_json::from_slice(&fs::read(file)?)?;
    if l.format != 1
        || !l.program_root.is_absolute()
        || !l.common_root.is_absolute()
        || l.legacy.as_ref().is_some_and(|p| !p.is_absolute())
    {
        return Err("invalid installation locator".into());
    }
    kavychka_storage::preview::no_link_chain(&l.program_root)?;
    kavychka_storage::preview::no_link_chain(&l.common_root)?;
    if !l.common_root.is_dir()
        || l.program_root
            .ancestors()
            .last()
            .is_none_or(|p| !p.is_dir())
    {
        return Err("Диск прежней установки или данных недоступен; подключи его. Пустая библиотека не создавалась. / Reconnect the installation or data disk; no empty fallback library was created.".into());
    }
    Ok(Some(l))
}
pub fn register_locator(local: &Path, root: &Path, current: &Current) -> Result<()> {
    validate(current)?;
    if fs::read(root.join("install.owner"))? != b"Kavychka.Next.v1\n" {
        return Err("unknown installation owner".into());
    }
    atomic(
        &local.join("Kavychka-next-install.json"),
        &Locator {
            format: 1,
            program_root: root.to_path_buf(),
            common_root: current.common.clone(),
            legacy: current.legacy.clone(),
        },
    )
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub format: u32,
    pub state: String,
    pub candidate: Current,
    pub previous: Option<Current>,
    pub backup: Option<PathBuf>,
    pub created: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_limit")]
    pub recovery_limit: u64,
}
fn default_limit() -> u64 {
    2_000_000_000
}
fn resolved(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut tail = vec![];
    let mut at = absolute.as_path();
    while !at.try_exists()? {
        tail.push(
            at.file_name()
                .ok_or("storage drive unavailable")?
                .to_os_string(),
        );
        at = at.parent().ok_or("storage drive unavailable")?;
    }
    let mut out = at.canonicalize()?;
    for part in tail.into_iter().rev() {
        out.push(part);
    }
    Ok(out)
}
fn disjoint(root: &Path, common: &Path) -> Result<()> {
    let root = resolved(root)?;
    let common = resolved(common)?;
    if root.starts_with(&common) || common.starts_with(&root) {
        return Err("program and shared storage must be separate".into());
    }
    Ok(())
}
pub fn request_recovery_limit(root: &Path, common: &Path, bytes: u64) -> Result<()> {
    let options = kavychka_core::preferences::DesktopOptions {
        recovery_limit_bytes: bytes,
        ..Default::default()
    };
    options
        .validate()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    if !common.is_absolute() {
        return Err("absolute shared folder required".into());
    }
    disjoint(root, common)?;
    let _guard = owned(root)?;
    let path = journal_path(root);
    if path.exists() {
        let mut j: Journal = serde_json::from_slice(&fs::read(&path)?)?;
        if j.format != 1 || resolved(&j.candidate.common)? != resolved(common)? {
            return Err("finish the previous transaction before choosing another storage".into());
        }
        j.recovery_limit = bytes;
        atomic(&path, &j)?;
    }
    atomic(
        &root.join("recovery-budget.json"),
        &serde_json::json!({"format":1,"common":common,"bytes":bytes}),
    )
}
pub struct Guard {
    _lock: std::fs::File,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupManifest {
    format: u32,
    files: Vec<package::Entry>,
    #[serde(default = "checked_default")]
    sqlite_checked: bool,
}
fn checked_default() -> bool {
    true
}
fn backup_manifest(root: &Path) -> Result<Vec<package::Entry>> {
    fn walk(base: &Path, p: &Path, out: &mut Vec<package::Entry>) -> Result<()> {
        for f in fs::read_dir(p)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            if f.file_type()?.is_dir() {
                walk(base, &f.path(), out)?
            } else {
                let name = f
                    .path()
                    .strip_prefix(base)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if name == "backup.manifest.json" {
                    continue;
                }
                out.push(package::Entry {
                    path: name,
                    bytes: f.metadata()?.len(),
                    sha256: crate::digest(&f.path())?,
                });
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
fn verify_backup(from: &Path) -> Result<()> {
    verify_rescue(from, true)
}
fn verify_rescue(from: &Path, require_sqlite: bool) -> Result<()> {
    kavychka_storage::preview::no_link_chain(from)?;
    let b = clean_path(from, "backup.manifest.json")?;
    if b.metadata()?.len() > 10_000_000 {
        return Err("backup manifest too large".into());
    }
    let m: BackupManifest = serde_json::from_slice(&fs::read(b)?)?;
    if m.format != 1
        || require_sqlite && !m.sqlite_checked
        || !m.files.iter().any(|f| f.path == "shell.sqlite3")
        || !m.files.iter().any(|f| f.path == "shell-sandbox.owner")
    {
        return Err("incomplete recovery backup".into());
    }
    if serde_json::to_value(&m.files)? != serde_json::to_value(backup_manifest(from)?)? {
        return Err("recovery backup integrity check failed; current data retained".into());
    }
    Ok(())
}
pub fn owned(root: &Path) -> Result<Guard> {
    if !root.is_absolute() {
        return Err("absolute install root required".into());
    }
    kavychka_storage::preview::no_link_chain(root)?;
    if !root.exists() {
        fs::create_dir_all(root)?
    }
    let marker = root.join("install.owner");
    if marker.exists() {
        if fs::read(&marker)? != b"Kavychka.Next.v1\n" {
            return Err("unknown installation owner".into());
        }
    } else {
        if fs::read_dir(root)?.next().is_some() {
            return Err("choose empty folder or owned Kavychka Next installation".into());
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)?;
        f.write_all(b"Kavychka.Next.v1\n")?;
        f.sync_all()?;
    }
    let p = clean_path(root, "install.lock")?;
    let lock = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .truncate(false)
        .open(p)?;
    lock.try_lock()
        .map_err(|_| "another installation is in progress")?;
    let ignore = clean_path(root, ".gitignore")?;
    if !ignore.exists() {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ignore)?;
        f.write_all(b"*\n")?;
        f.sync_all()?;
    }
    for d in ["versions", "recovery", "updates"] {
        let p = clean_path(root, d)?;
        fs::create_dir_all(p)?;
    }
    Ok(Guard { _lock: lock })
}
pub fn current(root: &Path) -> Result<Option<Current>> {
    let p = clean_path(root, "current.json")?;
    if !p.exists() {
        return Ok(None);
    }
    let c: Current = serde_json::from_slice(&fs::read(p)?)?;
    validate(&c)?;
    Ok(Some(c))
}
fn validate(c: &Current) -> Result<()> {
    package::version(&c.version)?;
    if c.format != 1
        || !c.common.is_absolute()
        || !c.data.is_absolute()
        || c.data != c.common.join(".kavychka-next")
    {
        return Err("invalid installation data pointer".into());
    }
    kavychka_storage::preview::no_link_chain(&c.common)?;
    kavychka_storage::preview::no_link_chain(&c.data)?;
    if let Some(source) = &c.legacy {
        if !source.is_absolute() {
            return Err("absolute legacy source required".into());
        }
        kavychka_storage::preview::no_link_chain(source)?;
    }
    Ok(())
}
fn list(root: &Path) -> Result<Vec<(String, u64)>> {
    fn walk(root: &Path, p: &Path, out: &mut Vec<(String, u64)>) -> Result<()> {
        for f in fs::read_dir(p)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            let name = f
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if p == root && excluded_data(&name) {
                continue;
            }
            if f.file_type()?.is_dir() {
                walk(root, &f.path(), out)?
            } else if !matches!(
                name.as_str(),
                "shell.sqlite3"
                    | "shell.sqlite3-wal"
                    | "shell.sqlite3-shm"
                    | "shell.sqlite3-journal"
            ) {
                out.push((name, f.metadata()?.len()))
            }
        }
        Ok(())
    }
    let mut out = vec![];
    if root.exists() {
        walk(root, root, &mut out)?;
    }
    Ok(out)
}
pub fn size(root: &Path) -> Result<u64> {
    fn walk(p: &Path) -> Result<u64> {
        let mut n = 0;
        if !p.exists() {
            return Ok(0);
        }
        for f in fs::read_dir(p)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            n += if f.file_type()?.is_dir() {
                walk(&f.path())?
            } else {
                f.metadata()?.len()
            };
        }
        Ok(n)
    }
    walk(root)
}
fn room(root: &Path, data: &Path, limit: u64) -> Result<()> {
    let owned = size(&root.join("recovery"))?
        + size(&data.join("quarantine"))?
        + size(&data.join("recovery"))?;
    let source = list(data)?.iter().map(|(_, n)| *n).sum::<u64>()
        + fs::metadata(data.join("shell.sqlite3"))
            .map(|m| m.len())
            .unwrap_or(0)
        + fs::metadata(data.join("shell.sqlite3-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
    if owned.saturating_add(source) > limit {
        return Err("Недостаточно лимита резервов: увеличь лимит или освободи место. Последняя копия сохранена. / Recovery budget exceeded; increase the limit or free space.".into());
    }
    crate::windows::disk_space(root, source.saturating_mul(2).saturating_add(50_000_000))?;
    Ok(())
}
pub fn snapshot(data: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        return Err("backup destination must be new".into());
    }
    let store = kavychka_storage::sandbox::Sandbox::open(data)?;
    let files = list(data)?;
    fs::create_dir(to)?;
    for (name, _) in files {
        let a = clean_path(data, &name)?;
        let b = clean_path(to, &name)?;
        fs::create_dir_all(b.parent().unwrap())?;
        fs::copy(a, &b)?;
        if crate::digest(&b)? != crate::digest(&clean_path(data, &name)?)? {
            return Err("backup file changed".into());
        }
    }
    store.backup_database(&to.join("shell.sqlite3"))?;
    atomic(
        &to.join("backup.manifest.json"),
        &BackupManifest {
            format: 1,
            files: backup_manifest(to)?,
            sqlite_checked: true,
        },
    )?;
    verify_backup(to)?;
    Ok(())
}
fn snapshot_forensic(data: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        return Err("forensic destination must be new".into());
    }
    kavychka_storage::preview::no_link_chain(data)?;
    if fs::read(data.join("shell-sandbox.owner"))? != b"kavychka-shell-sandbox-v1\n" {
        return Err("unknown data owner; not resetting it".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.join("shell.lock"))?;
    lock.try_lock()
        .map_err(|_| "close Kavychka before recovery")?;
    let mut files = list(data)?;
    for name in [
        "shell.sqlite3",
        "shell.sqlite3-wal",
        "shell.sqlite3-shm",
        "shell.sqlite3-journal",
    ] {
        let p = clean_path(data, name)?;
        if p.is_file() {
            files.push((name.into(), p.metadata()?.len()))
        }
    }
    fs::create_dir(to)?;
    for (name, _) in files {
        let source = clean_path(data, &name)?;
        let target = clean_path(to, &name)?;
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(&source, &target)?;
        if crate::digest(&source)? != crate::digest(&target)? {
            return Err("forensic source changed".into());
        }
    }
    atomic(
        &to.join("backup.manifest.json"),
        &BackupManifest {
            format: 1,
            files: backup_manifest(to)?,
            sqlite_checked: false,
        },
    )?;
    verify_rescue(to, false)
}
fn restore(data: &Path, from: &Path) -> Result<()> {
    verify_backup(from)?;
    if fs::read(from.join("shell-sandbox.owner"))? != b"kavychka-shell-sandbox-v1\n" {
        return Err("invalid recovery copy".into());
    }
    // Hold the same lock as the app, then restore only generation-owned content.
    let lock = OpenOptions::new()
        .write(true)
        .read(true)
        .open(data.join("shell.lock"))?;
    lock.try_lock()
        .map_err(|_| "close Kavychka before recovery")?;
    if fs::read(data.join("shell-sandbox.owner"))? != b"kavychka-shell-sandbox-v1\n" {
        return Err("invalid data owner".into());
    }
    for f in fs::read_dir(data)? {
        let f = f?;
        let name = f.file_name();
        if name.to_str().is_some_and(excluded_data) {
            continue;
        }
        if f.file_type()?.is_dir() {
            crate::delete_owned_tree(&f.path(), data)?
        } else {
            kavychka_storage::preview::no_link_chain(&f.path())?;
            fs::remove_file(f.path())?
        }
    }
    fn copy(from: &Path, to: &Path, top: bool) -> Result<()> {
        fs::create_dir_all(to)?;
        for f in fs::read_dir(from)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            let out = to.join(f.file_name());
            if f.file_name() == "backup.manifest.json"
                || top && f.file_name().to_str().is_some_and(excluded_data)
            {
                continue;
            }
            if f.file_type()?.is_dir() {
                copy(&f.path(), &out, false)?
            } else {
                fs::copy(f.path(), out)?;
            }
        }
        Ok(())
    }
    copy(from, data, true)?;
    Ok(())
}
fn journal_path(root: &Path) -> PathBuf {
    root.join("transaction.json")
}
pub fn recover(root: &Path) -> Result<()> {
    crate::relocate::recover(root)?;
    let p = journal_path(root);
    kavychka_storage::preview::no_link_chain(&p)?;
    if !p.exists() {
        return Ok(());
    }
    let mut j: Journal = serde_json::from_slice(&fs::read(&p)?)?;
    validate(&j.candidate)?;
    disjoint(root, &j.candidate.common)?;
    if j.format != 1 {
        return Err("unsupported install journal".into());
    }
    if matches!(j.state.as_str(), "complete" | "rolled_back") {
        return Ok(());
    }
    if !matches!(
        j.state.as_str(),
        "prepared" | "probing" | "activating" | "rolling_back"
    ) {
        return Err("unknown installation state; retained for review".into());
    }
    if let Some(prev) = &j.previous {
        validate(prev)?;
        if prev.data != j.candidate.data {
            return Err("update changed data root".into());
        }
        package::verify_installed(&root.join("versions").join(&prev.version))?;
    }
    if let Some(backup) = &j.backup {
        let parent = backup.parent().ok_or("invalid backup")?;
        if parent.canonicalize()? != root.join("recovery").canonicalize()?
            || !backup
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with("before-")
        {
            return Err("invalid recovery location".into());
        }
        verify_backup(backup)?;
        if j.candidate.data.exists() {
            let preserved = root
                .join("recovery")
                .join(format!("after-failed-{}", j.created.timestamp_millis()));
            if !preserved.exists() || verify_rescue(&preserved, false).is_err() {
                if preserved.exists() {
                    crate::delete_owned_tree(&preserved, &root.join("recovery"))?;
                }
                room(root, &j.candidate.data, j.recovery_limit)?;
                if snapshot(&j.candidate.data, &preserved).is_err() {
                    if preserved.exists() {
                        crate::delete_owned_tree(&preserved, &root.join("recovery"))?;
                    }
                    snapshot_forensic(&j.candidate.data, &preserved)?;
                }
            }
        }
        restore(&j.candidate.data, backup)?;
        crate::relocate::rebase_restored(root, &j.candidate.data)?;
    }
    match &j.previous {
        Some(prev) => atomic(&root.join("current.json"), prev)?,
        None => {
            let pointer = root.join("current.json");
            if pointer.exists() {
                fs::remove_file(pointer)?;
            }
        }
    }
    j.state = "rolled_back".into();
    atomic(&p, &j)?;
    Ok(())
}
pub fn apply(
    root: &Path,
    common: &Path,
    setup: &Path,
    probe: impl FnMut(&Path, &Current) -> Result<()>,
) -> Result<Current> {
    apply_with_source(root, common, setup, None, probe)
}
pub fn apply_with_source(
    root: &Path,
    common: &Path,
    setup: &Path,
    legacy: Option<&Path>,
    mut probe: impl FnMut(&Path, &Current) -> Result<()>,
) -> Result<Current> {
    let _guard = owned(root)?;
    disjoint(root, common)?;
    recover(root)?;
    let initially_empty_common = !common.try_exists()? || fs::read_dir(common)?.next().is_none();
    let budget_file = root.join("recovery-budget.json");
    if budget_file.exists() {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&budget_file)?)?;
        if v["format"] != 1
            || v["common"]
                .as_str()
                .map(Path::new)
                .map(resolved)
                .transpose()?
                .as_deref()
                != Some(resolved(common)?.as_path())
        {
            return Err("recovery budget belongs to another storage".into());
        }
        let bytes = v["bytes"].as_u64().ok_or("invalid recovery budget")?;
        let mut store = kavychka_storage::sandbox::Sandbox::open(&common.join(".kavychka-next"))?;
        store.set_recovery_limit(bytes)?;
        drop(store);
        fs::remove_file(budget_file)?;
    }
    if !common.is_absolute() {
        return Err("absolute shared storage required".into());
    }
    kavychka_storage::preview::no_link_chain(common)?;
    let manifest = package::manifest(setup)?;
    let old = current(root)?;
    if let Some(old) = &old {
        if old.common != common {
            return Err("update must keep the existing data folder".into());
        }
        if package::version(&manifest.version)? <= package::version(&old.version)? {
            return Err("update must be newer; use explicit rollback for an older version".into());
        }
    }
    if initially_empty_common {
        fs::create_dir_all(common)?;
        let marker = common.join("storage-owner.json");
        kavychka_storage::preview::no_link_chain(&marker)?;
        if !marker.exists() {
            atomic(
                &marker,
                &serde_json::json!({"application":"Kavychka","version":1}),
            )?;
        }
    }
    let candidate = Current {
        format: 1,
        version: manifest.version.clone(),
        common: common.to_path_buf(),
        data: common.join(".kavychka-next"),
        legacy: old
            .as_ref()
            .and_then(|c| c.legacy.clone())
            .or_else(|| legacy.map(Path::to_path_buf)),
    };
    validate(&candidate)?;
    if root.starts_with(common) || common.starts_with(root) {
        return Err("program and shared storage must be separate".into());
    }
    let version_root = root.join("versions").join(&manifest.version);
    if version_root.exists() {
        let installed = package::verify_installed(&version_root)?;
        if serde_json::to_value(&installed)? != serde_json::to_value(&manifest)? {
            return Err("different build already exists for this version".into());
        }
    } else {
        let stage = root
            .join("versions")
            .join(format!("stage-{}", manifest.version));
        if stage.exists() {
            crate::delete_owned_tree(&stage, &root.join("versions"))?;
        }
        crate::windows::disk_space(
            root,
            manifest.files.iter().map(|f| f.bytes).sum::<u64>() + 50_000_000,
        )?;
        package::extract(setup, &stage)?;
        fs::rename(stage, &version_root)?;
    }
    let backup = if let Some(old) = &old {
        let prefs = kavychka_storage::sandbox::Sandbox::open(&old.data)?.load()?;
        room(root, &old.data, prefs.desktop.recovery_limit_bytes)?;
        let path = root.join("recovery").join(format!(
            "before-{}-{}",
            old.version,
            chrono::Utc::now().timestamp_millis()
        ));
        snapshot(&old.data, &path)?;
        register_backup(root, &path, &old.version, chrono::Utc::now())?;
        Some(path)
    } else {
        None
    };
    let mut journal = Journal {
        format: 1,
        state: "prepared".into(),
        candidate: candidate.clone(),
        previous: old.clone(),
        backup,
        created: chrono::Utc::now(),
        recovery_limit: if let Some(old) = &old {
            kavychka_storage::sandbox::Sandbox::open(&old.data)?
                .load()?
                .desktop
                .recovery_limit_bytes
        } else {
            default_limit()
        },
    };
    atomic(&journal_path(root), &journal)?;
    let attempt = (|| {
        journal.state = "probing".into();
        atomic(&journal_path(root), &journal)?;
        probe(&version_root, &candidate)?;
        {
            let data = kavychka_storage::sandbox::Sandbox::open(&candidate.data)?;
            data.put_meta("recovery_path", &serde_json::json!(root.join("recovery")))?;
        }
        // Stable launcher delegates through current.json; it has no versioned settings.
        let launcher = root.join("Kavychka.exe");
        if !launcher.exists() {
            fs::copy(version_root.join("Kavychka.exe"), &launcher)?;
        }
        journal.state = "activating".into();
        atomic(&journal_path(root), &journal)?;
        atomic(&root.join("current.json"), &candidate)?;
        journal.state = "complete".into();
        atomic(&journal_path(root), &journal)?;
        Ok(candidate.clone())
    })();
    if attempt.is_err() {
        recover(root)?;
    } else if let Ok(store) = kavychka_storage::sandbox::Sandbox::open(&candidate.data) {
        let limit = store.load()?.desktop.recovery_limit_bytes;
        drop(store);
        if let Err(e) = prune_ready(root, limit) {
            std::fs::write(root.join("maintenance-warning.txt"), e.to_string())?;
        }
    }
    attempt
}
pub fn probe(version_root: &Path, current: &Current) -> Result<()> {
    let mut command = std::process::Command::new(version_root.join("app.exe"));
    command
        .arg("--prepare-install")
        .arg(&current.data)
        .arg("--common-root")
        .arg(&current.common)
        .arg("--expected-version")
        .arg(&current.version);
    if let Some(legacy) = &current.legacy {
        command.arg("--legacy-source").arg(legacy);
    }
    crate::windows::hide(&mut command);
    let log = current.common.join(".kavychka-install-check.log");
    fs::create_dir_all(&current.common)?;
    command
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::OpenOptions::new().append(true).open(&log)?);
    let mut child = command.spawn()?;
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(format!("new version check failed; see {}", log.display()).into());
            }
            break;
        }
        if started.elapsed() > std::time::Duration::from_secs(180) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("new version startup check timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}
pub fn rollback_available(root: &Path) -> bool {
    let Ok(bytes) = fs::read(journal_path(root)) else {
        return false;
    };
    let Ok(j) = serde_json::from_slice::<Journal>(&bytes) else {
        return false;
    };
    j.state == "complete"
        && j.created + chrono::Duration::days(7) >= chrono::Utc::now()
        && (j.previous.is_some()
            || j.candidate.legacy.is_some()
            || j.candidate.common.join("settings.json").is_file()
            || j.candidate.common.join("library.sqlite3").is_file())
}
pub fn rollback(root: &Path) -> Result<Option<Current>> {
    rollback_with_platform(root, |_| Ok(()))
}
pub fn rollback_with_platform(
    root: &Path,
    restore_legacy: impl FnOnce(&Path) -> Result<()>,
) -> Result<Option<Current>> {
    let _guard = owned(root)?;
    recover(root)?;
    let mut j: Journal = serde_json::from_slice(&fs::read(journal_path(root))?)?;
    if !rollback_available(root) {
        return Err("no verified rollback within 7 days".into());
    }
    if j.previous.is_none() {
        let data = kavychka_storage::sandbox::Sandbox::open(&j.candidate.data)?;
        drop(data);
        restore_legacy(root)?;
    }
    j.state = "rolling_back".into();
    atomic(&journal_path(root), &j)?;
    recover(root)?;
    current(root)
}
pub fn prune(root: &Path, limit: u64) -> Result<()> {
    let _guard = owned(root)?;
    prune_ready(root, limit)
}
fn prune_ready(root: &Path, limit: u64) -> Result<()> {
    prune_ready_at(root, limit, chrono::Utc::now())
}
pub(crate) fn prune_ready_at(
    root: &Path,
    limit: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let j: Journal = serde_json::from_slice(&fs::read(journal_path(root))?)?;
    if j.state != "complete" {
        return Ok(());
    }
    let mut index = read_backup_index(root)?;
    index.backups.sort_by_key(|b| std::cmp::Reverse(b.sequence));
    let mut usable = vec![];
    for b in &index.backups {
        let path = clean_path(&root.join("recovery"), &b.directory)?;
        if path.is_dir()
            && crate::digest(&path.join("backup.manifest.json"))
                .is_ok_and(|sha| sha == b.manifest_sha256)
            && verify_backup(&path).is_ok()
        {
            usable.push(b.clone());
        }
    }
    let protected = usable.first().map(|b| b.directory.as_str());
    let protected_names: std::collections::BTreeSet<_> = usable
        .iter()
        .filter(|b| {
            protected == Some(b.directory.as_str())
                || j.backup.as_ref() == Some(&root.join("recovery").join(&b.directory))
        })
        .map(|b| b.directory.clone())
        .collect();
    let mut versions: std::collections::BTreeSet<_> = usable
        .iter()
        .filter(|b| protected_names.contains(&b.directory))
        .map(|b| b.version.clone())
        .collect();
    let mut kept_versions = std::collections::BTreeSet::new();
    let mut deleted = std::collections::BTreeSet::new();
    for backup in &usable {
        let path = clean_path(&root.join("recovery"), &backup.directory)?;
        let protect = protected_names.contains(&backup.directory);
        let age = now.signed_duration_since(backup.created_utc);
        let keep_version = !kept_versions.contains(&backup.version)
            && (versions.contains(&backup.version) || versions.len() < 2);
        let keep = protect || (age <= chrono::Duration::days(7) && keep_version);
        if keep {
            versions.insert(backup.version.clone());
            kept_versions.insert(backup.version.clone());
        } else {
            crate::delete_owned_tree(&path, &root.join("recovery"))?;
            deleted.insert(backup.directory.clone());
        }
    }
    index.backups.retain(|b| !deleted.contains(&b.directory));
    atomic(&root.join("recovery-index.json"), &index)?;
    let total = size(&root.join("recovery"))?
        .saturating_add(size(&j.candidate.data.join("quarantine"))?)
        .saturating_add(size(&j.candidate.data.join("recovery"))?);
    if total > limit {
        return Err("recovery budget exceeded; last recovery copy retained".into());
    }
    Ok(())
}
fn excluded_data(name: &str) -> bool {
    matches!(
        name,
        "components"
            | "cache"
            | "jobs"
            | "shell.lock"
            | "recovery"
            | "exports"
            | "logs"
            | "downloads"
            | "updates"
            | "models"
            | "runtime"
    )
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupRecord {
    directory: String,
    version: String,
    created_utc: chrono::DateTime<chrono::Utc>,
    manifest_sha256: String,
    sequence: u64,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupIndex {
    format: u32,
    backups: Vec<BackupRecord>,
}
fn read_backup_index(root: &Path) -> Result<BackupIndex> {
    let file = clean_path(root, "recovery-index.json")?;
    if !file.exists() {
        return Ok(BackupIndex {
            format: 1,
            backups: vec![],
        });
    }
    if file.metadata()?.len() > 2_000_000 {
        return Err("recovery index is too large; copies retained".into());
    }
    let index: BackupIndex = serde_json::from_slice(&fs::read(file)?)?;
    if index.format != 1 || index.backups.len() > 1000 {
        return Err("unknown recovery index; copies retained".into());
    }
    let mut names = std::collections::BTreeSet::new();
    let mut sequences = std::collections::BTreeSet::new();
    for record in &index.backups {
        package::version(&record.version)?;
        if record.directory.contains(['/', '\\', ':'])
            || !record.directory.starts_with("before-")
            || !names.insert(&record.directory)
            || record.manifest_sha256.len() != 64
            || record.sequence == 0
            || !sequences.insert(record.sequence)
        {
            return Err("invalid recovery index; copies retained".into());
        }
        clean_path(&root.join("recovery"), &record.directory)?;
    }
    Ok(index)
}
pub(crate) fn register_backup(
    root: &Path,
    path: &Path,
    version: &str,
    created: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    verify_backup(path)?;
    package::version(version)?;
    if path
        .parent()
        .ok_or("missing recovery parent")?
        .canonicalize()?
        != root.join("recovery").canonicalize()?
    {
        return Err("backup is outside recovery owner".into());
    }
    let directory = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid backup name")?
        .to_owned();
    let mut index = read_backup_index(root)?;
    let sequence = index
        .backups
        .iter()
        .map(|b| b.sequence)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("recovery sequence overflow")?;
    index.backups.retain(|b| b.directory != directory);
    index.backups.push(BackupRecord {
        directory,
        version: version.into(),
        created_utc: created,
        manifest_sha256: crate::digest(&path.join("backup.manifest.json"))?,
        sequence,
    });
    atomic(&root.join("recovery-index.json"), &index)
}
