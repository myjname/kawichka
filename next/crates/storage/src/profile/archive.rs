use super::*;
use crate::{datafs, sandbox::Sandbox};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
};
const MANIFEST_LIMIT: u64 = 4 * 1024 * 1024;
struct PartialArchive(PathBuf);
impl Drop for PartialArchive {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    pub library: bool,
    pub audio: bool,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            library: true,
            audio: true,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub kind: String,
    pub app_version: String,
    pub working_schema: u32,
    pub created_utc: chrono::DateTime<chrono::Utc>,
    pub options: ExportOptions,
    pub entries: Vec<Entry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub phrases: usize,
    pub presets: usize,
    pub sha256: String,
    pub peak_bytes: u64,
}
/// Constructed only from an already owned store. The worker opens its own
/// read-only connection; it never acquires a second writer or opens legacy data.
pub struct CaptureSource {
    root: PathBuf,
    limit: u64,
}
impl CaptureSource {
    pub(crate) fn from_root(root: &Path, limit: u64) -> Self {
        Self {
            root: root.into(),
            limit,
        }
    }
    pub fn prepare_restore(
        self,
        inspected: Inspected,
        cancel: &AtomicBool,
    ) -> Result<super::RestorePlan> {
        let snapshot = super::snapshot::read(&self.root.join("shell.sqlite3"))?;
        super::RestorePlan::prepare_from(self.root, snapshot, inspected, cancel)
    }
    pub fn new(store: &Sandbox) -> Result<Self> {
        Ok(Self {
            root: store.root.clone(),
            limit: store.load()?.desktop.recovery_limit_bytes,
        })
    }
    pub fn estimate(&self, options: ExportOptions, cancel: &AtomicBool) -> Result<(u64, u64)> {
        let mut bytes = 0u64;
        for name in source_files(&self.root, options, cancel)? {
            bytes = bytes
                .checked_add(fs::metadata(crate::presets::inside(&self.root, &name)?)?.len())
                .ok_or_else(|| invalid("profile size overflow"))?;
        }
        for name in ["shell.sqlite3", "shell.sqlite3-wal"] {
            let path = self.root.join(name);
            crate::preview::no_link_chain(&path)?;
            if path.try_exists()? {
                bytes = bytes.saturating_add(path.metadata()?.len());
            }
        }
        if bytes > self.limit {
            return Err(invalid(
                "profile exceeds the configured data budget; increase the limit before exporting",
            ));
        }
        Ok((bytes, bytes.saturating_mul(3).saturating_add(64_000_000)))
    }
    pub fn export(
        self,
        output: &Path,
        job: &Path,
        options: ExportOptions,
        app_version: &str,
        cancel: &AtomicBool,
        progress: impl Fn(&str),
    ) -> Result<ExportReport> {
        if !output.is_absolute() || output.exists() || !job.is_absolute() || job.exists() {
            return Err(invalid("profile target and job must be new absolute paths"));
        }
        crate::preview::no_link_chain(output)?;
        crate::preview::no_link_chain(job)?;
        let (_, peak) = self.estimate(options, cancel)?;
        datafs::room(job, peak)?;
        datafs::room(output, peak)?;
        let _owned_job = OwnedJob::new(job, "export")?;
        let payload = job.join("payload");
        fs::create_dir(&payload)?;
        progress("snapshot");
        datafs::check(cancel)?;
        let raw = job.join("captured.sqlite3");
        {
            let source = self.root.join("shell.sqlite3");
            crate::preview::no_link_chain(&source)?;
            let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut target = Connection::open(&raw)?;
            let backup = Backup::new(&source, &mut target)?;
            loop {
                datafs::check(cancel)?;
                match backup.step(128)? {
                    StepResult::Done => break,
                    StepResult::More => (),
                    _ => return Err(Error::Busy),
                }
            }
        }
        let mut snapshot = super::snapshot::read(&raw)?;
        if !options.library {
            snapshot.phrases.clear();
            snapshot.claims.clear();
            snapshot.state = Default::default();
            snapshot.meta.retain(|k, _| {
                ![
                    "legacy_preparation_states",
                    "preparation_state",
                    "audio_day",
                    "support_day",
                    "support_history",
                ]
                .contains(&k.as_str())
                    && !k.starts_with("delivery_journal:")
                    && !k.starts_with("pending_notice:")
                    && !k.starts_with("purpose:")
                    && !k.starts_with("profile_history:")
                    && !k.starts_with("profile_restore:")
            });
            if let Some(legacy) = snapshot.meta.get_mut("legacy_settings")
                && let Some(desktop) = legacy.get_mut("desktop").and_then(|v| v.as_object_mut())
            {
                desktop.remove("History");
            }
        }
        if !options.audio {
            for p in &mut snapshot.phrases {
                p.audio_path = None;
            }
        }
        super::snapshot::write(&payload.join("library.sqlite3"), &snapshot)?;
        fs::remove_file(&raw)?;
        progress("files");
        for name in source_files(&self.root, options, cancel)? {
            let source = crate::presets::inside(&self.root, &name)?;
            let dest = crate::presets::inside(&payload, &name)?;
            let copied = datafs::copy(&source, &dest, cancel)?;
            if copied != datafs::hash(&source, cancel)? {
                return Err(Error::SourceChanged);
            }
        }
        // Metadata is regenerated from its typed form, so unknown credential
        // fields cannot survive an export as unparsed preset/settings JSON.
        let (presets, warnings) = crate::presets::scan(&payload)?;
        if !warnings.is_empty() {
            return Err(invalid("fix invalid presets before exporting"));
        }
        for p in &presets {
            datafs::json(
                &crate::presets::folder(&payload, &p.id)?.join("preset.json"),
                &p.preset,
            )?;
        }
        for name in datafs::tree(&payload.join("legacy"), cancel)? {
            if name.ends_with("library.snapshot.sqlite3") {
                sanitize_legacy(
                    &presets_path(&payload.join("legacy"), &name)?,
                    options.audio,
                )?;
            }
        }
        super::snapshot::assets(&payload, &snapshot)?;
        let mut entries = vec![];
        let mut bytes = 0u64;
        for name in datafs::tree(&payload, cancel)? {
            if !allowed(&name) {
                return Err(invalid("profile contains an unsupported file"));
            }
            let (size, sha256) = datafs::hash(&crate::presets::inside(&payload, &name)?, cancel)?;
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| invalid("profile size overflow"))?;
            if bytes > self.limit {
                return Err(invalid("profile exceeds configured data budget"));
            }
            entries.push(Entry {
                path: name,
                bytes: size,
                sha256,
            });
        }
        let manifest = Manifest {
            format: 1,
            kind: "kavychka-profile".into(),
            app_version: app_version.into(),
            working_schema: 3,
            created_utc: chrono::Utc::now(),
            options,
            entries,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        if manifest_bytes.len() as u64 > MANIFEST_LIMIT {
            return Err(invalid("profile manifest too large"));
        }
        progress("archive");
        fs::create_dir_all(
            output
                .parent()
                .ok_or_else(|| invalid("missing export parent"))?,
        )?;
        let temporary = output.with_file_name(format!("{}.part", datafs::id()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let partial = PartialArchive(temporary.clone());
        let mut zip = zip::ZipWriter::new(file);
        let settings = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        zip.start_file("manifest.json", settings)
            .map_err(zip_error)?;
        zip.write_all(&manifest_bytes)?;
        progress("archive-writing");
        let mut buffer = [0u8; 65536];
        for entry in &manifest.entries {
            datafs::check(cancel)?;
            zip.start_file(&entry.path, settings).map_err(zip_error)?;
            let mut source = File::open(crate::presets::inside(&payload, &entry.path)?)?;
            loop {
                datafs::check(cancel)?;
                let n = source.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n])?;
            }
        }
        zip.finish().map_err(zip_error)?.sync_all()?;
        datafs::check(cancel)?;
        let (archive_bytes, sha256) = datafs::hash(&temporary, cancel)?;
        // Reopen the actual ZIP and verify bytes/SQLite/references before it is
        // published, not just the payload used to construct it.
        let verify = job.join("verify");
        let verified = inspect(&temporary, &verify, self.limit, cancel, |_| {})?;
        if verified.manifest.entries != manifest.entries {
            return Err(invalid("export verification mismatch"));
        }
        datafs::publish(&temporary, output)?;
        drop(partial);
        progress("complete");
        Ok(ExportReport {
            path: output.into(),
            files: manifest.entries.len(),
            bytes: archive_bytes,
            phrases: snapshot.phrases.len(),
            presets: presets.len(),
            sha256,
            peak_bytes: peak,
        })
    }
}
fn source_files(root: &Path, options: ExportOptions, cancel: &AtomicBool) -> Result<Vec<String>> {
    let mut names = vec![];
    for dir in ["presets", "audio", "builtin-days"] {
        if dir == "audio" && (!options.library || !options.audio)
            || dir == "builtin-days" && !options.library
        {
            continue;
        }
        for name in datafs::tree(&root.join(dir), cancel)? {
            names.push(format!("{dir}/{name}"));
        }
    }
    if options.library {
        for name in [
            "legacy/library.snapshot.sqlite3",
            "legacy/reminder-state.json",
        ] {
            if root.join(name).try_exists()? {
                names.push(name.into());
            }
        }
    }
    if options.library {
        for name in datafs::tree(&root.join("legacy/imported"), cancel)? {
            names.push(format!("legacy/imported/{name}"));
        }
        for name in datafs::tree(&root.join("legacy/profiles"), cancel)? {
            names.push(format!("legacy/profiles/{name}"));
        }
    }
    for name in &names {
        if !allowed(name) {
            return Err(invalid(format!("unsupported profile file: {name}")));
        }
    }
    names.sort();
    Ok(names)
}
fn zip_error(e: zip::result::ZipError) -> Error {
    match e {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => invalid(format!("invalid profile ZIP: {other}")),
    }
}
pub fn inspect(
    archive: &Path,
    job: &Path,
    limit: u64,
    cancel: &AtomicBool,
    progress: impl Fn(&str),
) -> Result<Inspected> {
    crate::preview::no_link_chain(archive)?;
    crate::preview::no_link_chain(job)?;
    if !archive.is_file() || !job.is_absolute() || job.exists() {
        return Err(invalid(
            "choose an existing archive and a new inspection job",
        ));
    }
    let (archive_bytes, archive_sha256) = datafs::hash(archive, cancel)?;
    if archive_bytes > limit.saturating_add(MANIFEST_LIMIT) {
        return Err(invalid("compressed profile exceeds configured data budget"));
    }
    let mut zip = zip::ZipArchive::new(File::open(archive)?).map_err(zip_error)?;
    if zip.len() > 20001 {
        return Err(invalid("profile contains too many files"));
    }
    if !zip.file_names().any(|name| name == "manifest.json")
        && zip.file_names().any(|name| {
            name == "preset.json"
                || Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("kavychka"))
        })
    {
        return Err(invalid(
            "character preset archive; import in Character after setup",
        ));
    }
    let manifest: Manifest = {
        let mut file = zip.by_name("manifest.json").map_err(zip_error)?;
        if file.size() > MANIFEST_LIMIT {
            return Err(invalid("profile manifest too large"));
        }
        let mut bytes = vec![];
        file.by_ref()
            .take(MANIFEST_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)?
    };
    if manifest.format != 1
        || manifest.kind != "kavychka-profile"
        || manifest.working_schema != 3
        || manifest.app_version.is_empty()
        || manifest.app_version.len() > 80
        || manifest.entries.len() > 20000
    {
        return Err(invalid("unsupported profile format/version"));
    }
    let mut expected = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.entries {
        crate::presets::inside(job, &entry.path)?;
        if !allowed(&entry.path)
            || !expected.insert(entry.path.clone())
            || !folded.insert(entry.path.to_lowercase())
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid/duplicate profile entry"));
        }
        total = total
            .checked_add(entry.bytes)
            .ok_or_else(|| invalid("profile size overflow"))?;
        if total > limit {
            return Err(invalid("unpacked profile exceeds configured data budget"));
        }
    }
    if !expected.contains("library.sqlite3") || zip.len() != expected.len() + 1 {
        return Err(invalid("profile manifest/files disagree"));
    }
    let mut seen = BTreeSet::new();
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(zip_error)?;
        if file.is_dir()
            || file
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
            || !seen.insert(file.name().to_lowercase())
            || file.name() != "manifest.json" && !expected.contains(file.name())
        {
            return Err(invalid("unexpected/duplicate/linked profile file"));
        }
    }
    datafs::room(job, total.saturating_add(16_000_000))?;
    let owned_job = OwnedJob::new(job, "inspect")?;
    let payload = job.join("payload");
    fs::create_dir(&payload)?;
    progress("verify");
    for entry in &manifest.entries {
        datafs::check(cancel)?;
        let mut file = zip.by_name(&entry.path).map_err(zip_error)?;
        if file.size() != entry.bytes {
            return Err(invalid("profile entry size differs"));
        }
        let output = crate::presets::inside(&payload, &entry.path)?;
        fs::create_dir_all(output.parent().unwrap())?;
        let mut dest = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)?;
        let mut buffer = [0u8; 65536];
        let mut count = 0u64;
        loop {
            datafs::check(cancel)?;
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count = count.saturating_add(n as u64);
            if count > entry.bytes {
                return Err(invalid("profile entry expanded beyond its declared size"));
            }
            dest.write_all(&buffer[..n])?;
        }
        dest.sync_all()?;
        drop(dest);
        if datafs::hash(&output, cancel)? != (entry.bytes, entry.sha256.clone()) {
            return Err(invalid("profile entry SHA/size failed"));
        }
    }
    if datafs::hash(archive, cancel)? != (archive_bytes, archive_sha256.clone()) {
        return Err(Error::SourceChanged);
    }
    let snapshot = super::snapshot::read(&payload.join("library.sqlite3"))?;
    if !manifest.options.library && (!snapshot.phrases.is_empty() || !snapshot.claims.is_empty())
        || !manifest.options.audio && snapshot.phrases.iter().any(|p| p.audio_path.is_some())
    {
        return Err(invalid(
            "profile content contradicts its selected composition",
        ));
    }
    super::snapshot::assets(&payload, &snapshot)?;
    for name in datafs::tree(&payload.join("legacy"), cancel)? {
        if name.ends_with("library.snapshot.sqlite3") {
            validate_legacy(&presets_path(&payload.join("legacy"), &name)?)?;
        } else if name.ends_with("/library.sqlite3") {
            super::snapshot::read(&presets_path(&payload.join("legacy"), &name)?)?;
        }
    }
    progress("review");
    Ok(Inspected {
        _job: owned_job,
        root: payload,
        manifest,
        archive_sha256,
        snapshot,
    })
}
fn validate_legacy(path: &Path) -> Result<()> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.pragma_update(None, "trusted_schema", false)?;
    if db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))? != 3 {
        return Err(invalid("unsupported legacy profile snapshot"));
    }
    let mut q =
        db.prepare("SELECT name,type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")?;
    for row in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (name, kind, sql) = row?;
        if !["table", "index"].contains(&kind.as_str())
            || sql.to_ascii_lowercase().contains("virtual table")
            || kind == "table"
                && ![
                    "settings",
                    "phrases",
                    "plans",
                    "voice_rotation",
                    "preparation",
                    "app_settings",
                ]
                .contains(&name.as_str())
        {
            return Err(invalid("unsupported legacy profile schema"));
        }
    }
    let raw: String = db.query_row("SELECT json FROM app_settings WHERE id=1", [], |r| r.get(0))?;
    let snapshot: kavychka_core::legacy::SettingsSnapshot = serde_json::from_str(&raw)?;
    if snapshot.format != 1 {
        return Err(invalid("unsupported legacy settings format"));
    }
    let allowed = kavychka_core::legacy::reconcile(
        &snapshot.desktop.clone().into_iter().collect(),
        &snapshot.inference,
    )?;
    if snapshot.desktop != allowed.desktop || snapshot.inference != allowed.inference {
        return Err(invalid("legacy snapshot contains unsupported settings"));
    }
    let settings: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='settings')",
        [],
        |r| r.get(0),
    )?;
    if settings
        && db.query_row::<i64, _, _>("SELECT COUNT(*) FROM settings", [], |r| r.get(0))? != 0
    {
        return Err(invalid("legacy credential settings were not sanitized"));
    }
    if db.query_row::<String, _, _>("PRAGMA integrity_check", [], |r| r.get(0))? != "ok" {
        return Err(invalid("legacy snapshot integrity failed"));
    }
    Ok(())
}
fn sanitize_legacy(path: &Path, audio: bool) -> Result<()> {
    validate_legacy(path)?;
    let db = Connection::open(path)?;
    db.pragma_update(None, "secure_delete", true)?;
    if !audio {
        let phrases: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='phrases')",
            [],
            |r| r.get(0),
        )?;
        if phrases {
            db.execute("UPDATE phrases SET audio_path=NULL", [])?;
        }
    }
    db.execute_batch("VACUUM;")?;
    Ok(())
}
fn presets_path(root: &Path, name: &str) -> Result<PathBuf> {
    crate::presets::inside(root, name)
}
