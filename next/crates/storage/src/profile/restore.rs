use super::*;
use crate::{
    datafs,
    library::{self, Phrase},
    presets,
    sandbox::Sandbox,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub kind: String,
    pub id: String,
    pub resolution: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreSummary {
    pub archive_sha256: String,
    pub clean_profile: bool,
    pub already_imported: bool,
    pub phrases_added: usize,
    pub phrases_kept: usize,
    pub duplicate_texts: usize,
    pub presets_added: usize,
    pub presets_reused: usize,
    pub remapped_files: usize,
    pub files_added: usize,
    pub bytes_added: u64,
    pub backup_estimate_bytes: u64,
    pub peak_bytes: u64,
    pub conflicts: Vec<Conflict>,
    pub conflict_count: usize,
    pub unavailable_model_paths: Vec<String>,
    pub current_settings_preserved: bool,
}
struct FilePlan {
    source: String,
    target: String,
    bytes: u64,
    sha256: String,
    prepared: Option<PathBuf>,
}
pub struct RestorePlan {
    publication: Option<OwnedJob>,
    inspected: Inspected,
    target: PathBuf,
    fingerprint: String,
    rows: Vec<Phrase>,
    presets: BTreeMap<String, String>,
    files: Vec<FilePlan>,
    audio_map: BTreeMap<String, String>,
    summary: RestoreSummary,
    backup: Option<ExportReport>,
    backup_guard: Option<fs::File>,
    operation: String,
    limit: u64,
    external_recovery: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    pub phrases_added: usize,
    pub presets_added: usize,
    pub backup: Option<PathBuf>,
    pub warning: Option<String>,
    pub already_imported: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format: u32,
    archive: String,
    files: Vec<Publication>,
    dirs: Vec<String>,
    backup: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    entry: Entry,
    prepared: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn mapped_audio(root: &Path, mapping: &BTreeMap<String, String>, name: &str) -> Result<String> {
    if let Some(mapped) = mapping.get(name) {
        return Ok(mapped.clone());
    }
    #[cfg(windows)]
    {
        // Match the same existing file accepted by snapshot::assets, using the
        // filesystem's case/alias policy rather than Unicode text case folding.
        let actual = presets::inside(&root.join("audio"), name)?.canonicalize()?;
        if let Some(mapped) = actual
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| mapping.get(n))
        {
            return Ok(mapped.clone());
        }
    }
    #[cfg(not(windows))]
    let _ = root;
    Err(invalid("audio mapping missing"))
}
fn fingerprint(root: &Path, snapshot: &Snapshot, cancel: &AtomicBool) -> Result<String> {
    // A periodic checkpoint alone does not stale a reviewed plan. Actual user
    // preferences and reminder state must never be overwritten by an old plan.
    let mut state = snapshot.state.clone();
    state.checkpoint_utc = None;
    let mut data = serde_json::to_vec(
        &serde_json::json!({"prefs":snapshot.prefs,"state":state,"phrases":snapshot.phrases,"pending_archive":snapshot.meta.iter().filter(|(k,_)| k.starts_with("pending_notice:")).collect::<BTreeMap<_,_>>()}),
    )?;
    for dir in ["audio", "presets", "builtin-days", "legacy"] {
        for name in datafs::tree(&root.join(dir), cancel)? {
            data.extend_from_slice(format!("{dir}/{name}").as_bytes());
            let metadata = presets::inside(&root.join(dir), &name)?.metadata()?;
            data.extend_from_slice(
                serde_json::to_string(&(
                    metadata.len(),
                    metadata
                        .modified()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                ))?
                .as_bytes(),
            );
        }
    }
    Ok(digest(&data))
}
fn preset_hash(root: &Path, id: &str, cancel: &AtomicBool) -> Result<String> {
    let preset = presets::read(root, id)?;
    let mut bytes = serde_json::to_vec(&preset)?;
    for name in preset.files() {
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(
            datafs::hash(&presets::inside(&presets::folder(root, id)?, name)?, cancel)?
                .1
                .as_bytes(),
        );
    }
    Ok(digest(&bytes))
}
fn conflict(summary: &mut RestoreSummary, kind: &str, id: &str, resolution: &str) {
    summary.conflict_count += 1;
    if summary.conflicts.len() < 50 {
        summary.conflicts.push(Conflict {
            kind: kind.into(),
            id: id.into(),
            resolution: resolution.into(),
        });
    }
}
impl RestorePlan {
    pub fn prepare(store: &Sandbox, inspected: Inspected, cancel: &AtomicBool) -> Result<Self> {
        Self::prepare_from(
            store.root.clone(),
            super::snapshot::current(store)?,
            inspected,
            cancel,
        )
    }
    pub(crate) fn prepare_from(
        target: PathBuf,
        current: Snapshot,
        inspected: Inspected,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        inspected.validate_files(cancel)?;
        let (current_presets, warnings) = presets::scan(&target)?;
        if !warnings.is_empty() {
            return Err(invalid("fix invalid current presets before restoring"));
        }
        let mut meaningful_state = current.state.clone();
        meaningful_state.checkpoint_utc = None;
        let defaults = kavychka_core::preferences::Preferences::default();
        let mut settings = current.prefs.clone();
        // The chosen local backup budget is deliberately retained on clean
        // restore and must not make otherwise untouched defaults nonempty.
        settings.desktop.recovery_limit_bytes = defaults.desktop.recovery_limit_bytes;
        let untouched_defaults = serde_json::to_value(settings)? == serde_json::to_value(defaults)?;
        let empty_state = serde_json::to_value(&meaningful_state)?
            == serde_json::to_value(kavychka_core::engine::State::default())?;
        let clean = (!current.initialized || untouched_defaults)
            && empty_state
            && current.phrases.is_empty()
            && current_presets.is_empty()
            && current.state.pending.is_empty()
            && !current
                .meta
                .keys()
                .any(|k| k.starts_with("pending_notice:"));
        let already = current
            .meta
            .contains_key(&format!("profile_restore:{}", inspected.archive_sha256));
        let mut summary = RestoreSummary {
            archive_sha256: inspected.archive_sha256.clone(),
            clean_profile: clean,
            already_imported: already,
            phrases_added: 0,
            phrases_kept: 0,
            duplicate_texts: 0,
            presets_added: 0,
            presets_reused: 0,
            remapped_files: 0,
            files_added: 0,
            bytes_added: 0,
            backup_estimate_bytes: 0,
            peak_bytes: 0,
            conflicts: vec![],
            conflict_count: 0,
            unavailable_model_paths: vec![],
            current_settings_preserved: !clean,
        };
        let mut preset_map = BTreeMap::new();
        let mut selected_presets = BTreeMap::<String, String>::new();
        if !already {
            for entry in presets::list(&inspected.root)? {
                let source_hash = preset_hash(&inspected.root, &entry.id, cancel)?;
                let mut selected = entry.id.clone();
                if current_presets.iter().any(|p| p.id == selected) {
                    if preset_hash(&target, &selected, cancel)? == source_hash {
                        summary.presets_reused += 1;
                    } else {
                        selected = format!("r{source_hash}");
                        conflict(
                            &mut summary,
                            "preset",
                            &entry.id,
                            "new ID; current preset retained",
                        );
                    }
                }
                if selected != entry.id
                    && presets::folder(&target, &selected)?.exists()
                    && preset_hash(&target, &selected, cancel)? != source_hash
                {
                    return Err(invalid(
                        "restored preset ID conflicts with modified local data",
                    ));
                }
                if selected_presets
                    .get(&selected)
                    .is_some_and(|hash| hash != &source_hash)
                {
                    selected = format!(
                        "r{}",
                        digest(
                            format!("{}:{}:{source_hash}", inspected.archive_sha256, entry.id)
                                .as_bytes()
                        )
                    );
                    if presets::folder(&target, &selected)?.exists()
                        && preset_hash(&target, &selected, cancel)? != source_hash
                        || selected_presets
                            .get(&selected)
                            .is_some_and(|hash| hash != &source_hash)
                    {
                        return Err(invalid("preset identity collision; no data applied"));
                    }
                    conflict(
                        &mut summary,
                        "preset",
                        &entry.id,
                        "new unique ID for an incoming alias collision",
                    );
                }
                if !presets::folder(&target, &selected)?.exists()
                    && !selected_presets.contains_key(&selected)
                {
                    summary.presets_added += 1;
                }
                selected_presets.insert(selected.clone(), source_hash);
                preset_map.insert(entry.id, selected);
            }
        }
        let mut files = vec![];
        let mut audio = BTreeMap::new();
        let mut planned = BTreeSet::new();
        if !already {
            for entry in &inspected.manifest.entries {
                if entry.path == "library.sqlite3" {
                    if !clean {
                        let dest = format!(
                            "legacy/profiles/{}/library.sqlite3",
                            inspected.archive_sha256
                        );
                        let existing = presets::inside(&target, &dest)?;
                        if !existing.exists() {
                            summary.bytes_added = summary.bytes_added.saturating_add(entry.bytes);
                            files.push(FilePlan {
                                source: entry.path.clone(),
                                target: dest,
                                bytes: entry.bytes,
                                sha256: entry.sha256.clone(),
                                prepared: None,
                            });
                        } else if datafs::hash(&existing, cancel)?
                            != (entry.bytes, entry.sha256.clone())
                        {
                            return Err(invalid("archived profile snapshot was changed"));
                        }
                    }
                    continue;
                }
                let mut dest = entry.path.clone();
                if let Some(rest) = dest.strip_prefix("presets/") {
                    let (id, tail) = rest
                        .split_once('/')
                        .ok_or_else(|| invalid("invalid preset path"))?;
                    dest = format!(
                        "presets/{}/{tail}",
                        preset_map
                            .get(id)
                            .ok_or_else(|| invalid("unrecognized preset folder"))?
                    );
                }
                let target_file = presets::inside(&target, &dest)?;
                if target_file.exists() {
                    if datafs::hash(&target_file, cancel)? == (entry.bytes, entry.sha256.clone()) {
                        if let Some(name) = entry.path.strip_prefix("audio/") {
                            audio.insert(
                                name.to_owned(),
                                dest.strip_prefix("audio/").unwrap().to_owned(),
                            );
                        }
                        continue;
                    }
                    let name = Path::new(&entry.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| invalid("invalid asset filename"))?;
                    if entry.path.starts_with("audio/") || entry.path.starts_with("builtin-days/") {
                        let ext = Path::new(name)
                            .extension()
                            .and_then(|n| n.to_str())
                            .ok_or_else(|| invalid("asset has no extension"))?;
                        dest = format!(
                            "{}/r{}.{}",
                            entry.path.split('/').next().unwrap(),
                            entry.sha256,
                            ext.to_ascii_lowercase()
                        );
                    } else if entry.path.starts_with("legacy/") {
                        dest = format!("legacy/imported/{}/{name}", inspected.archive_sha256);
                    } else {
                        return Err(invalid("preset file collision after ID mapping"));
                    }
                    summary.remapped_files += 1;
                    conflict(
                        &mut summary,
                        "file",
                        &entry.path,
                        "content-addressed name; current file retained",
                    );
                }
                if let Some(name) = entry.path.strip_prefix("audio/") {
                    audio.insert(name.into(), dest.strip_prefix("audio/").unwrap().into());
                }
                let target_file = presets::inside(&target, &dest)?;
                if target_file.exists() {
                    if datafs::hash(&target_file, cancel)? != (entry.bytes, entry.sha256.clone()) {
                        return Err(invalid("content-addressed file was modified"));
                    }
                } else if planned.insert(dest.clone()) {
                    summary.bytes_added = summary.bytes_added.saturating_add(entry.bytes);
                    files.push(FilePlan {
                        source: entry.path.clone(),
                        target: dest,
                        bytes: entry.bytes,
                        sha256: entry.sha256.clone(),
                        prepared: None,
                    });
                }
            }
        }
        let current_ids: BTreeSet<_> = current.phrases.iter().map(|p| p.id.as_str()).collect();
        let mut known: BTreeSet<_> = current
            .phrases
            .iter()
            .map(|p| {
                (
                    p.text.to_lowercase(),
                    p.category.clone(),
                    p.language.clone(),
                    p.followup,
                )
            })
            .collect();
        let mut rows = vec![];
        if !already {
            for source in &inspected.snapshot.phrases {
                if current_ids.contains(source.id.as_str()) {
                    summary.phrases_kept += 1;
                    conflict(
                        &mut summary,
                        "phrase",
                        &source.id,
                        "current row and exclusions retained",
                    );
                    continue;
                }
                let key = (
                    source.text.to_lowercase(),
                    source.category.clone(),
                    source.language.clone(),
                    source.followup,
                );
                if !clean && !known.insert(key) {
                    summary.duplicate_texts += 1;
                    continue;
                }
                let mut row = source.clone();
                if let Some(path) = &row.audio_path {
                    row.audio_path = Some(mapped_audio(&inspected.root, &audio, path)?);
                }
                if let Some(id) = preset_map.get(&row.profile) {
                    row.profile = id.clone();
                }
                for (old, new) in &preset_map {
                    if row.voice_reference == *old {
                        row.voice_reference = new.clone();
                    }
                }
                rows.push(row);
            }
        }
        summary.phrases_added = rows.len();
        summary.files_added = files.len();
        summary.unavailable_model_paths = inspected
            .snapshot
            .prefs
            .generation
            .model_paths
            .iter()
            .filter(|(_, p)| !Path::new(p).exists())
            .map(|(id, _)| id.clone())
            .collect();
        let (_, peak) = super::archive::CaptureSource::from_root(
            &target,
            current.prefs.desktop.recovery_limit_bytes,
        )
        .estimate(ExportOptions::default(), cancel)?;
        summary.backup_estimate_bytes = peak.saturating_sub(64_000_000) / 3 + MANIFEST_MARGIN;
        summary.peak_bytes = peak
            .saturating_add(summary.bytes_added)
            .saturating_add(summary.backup_estimate_bytes);
        let external_recovery = current
            .meta
            .get("recovery_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let limit = current.prefs.desktop.recovery_limit_bytes;
        Ok(Self {
            publication: None,
            fingerprint: fingerprint(&target, &current, cancel)?,
            inspected,
            target,
            rows,
            presets: preset_map,
            files,
            audio_map: audio,
            summary,
            backup: None,
            backup_guard: None,
            operation: datafs::id(),
            limit,
            external_recovery,
        })
    }
    pub fn summary(&self) -> &RestoreSummary {
        &self.summary
    }
    pub fn prepare_files(
        &mut self,
        source: CaptureSource,
        app_version: &str,
        cancel: &AtomicBool,
        progress: impl Fn(&str),
    ) -> Result<()> {
        if self.summary.already_imported {
            return Ok(());
        }
        let backup_root = self.target.join("recovery/profiles").join(&self.operation);
        fs::create_dir_all(&backup_root)?;
        let mut reserved = 0u64;
        let mut roots = vec![self.target.join("recovery"), self.target.join("quarantine")];
        if let Some(external) = &self.external_recovery {
            if !external.is_dir() {
                return Err(invalid(
                    "recovery storage unavailable; reconnect it before restoring",
                ));
            }
            if external != &self.target.join("recovery") {
                roots.push(external.clone());
            }
        }
        for root in roots {
            for name in datafs::tree(&root, cancel)? {
                reserved =
                    reserved.saturating_add(presets::inside(&root, &name)?.metadata()?.len());
            }
        }
        if reserved.saturating_add(self.summary.backup_estimate_bytes) > self.limit {
            return Err(invalid(
                "recovery/quarantine budget cannot fit a safe pre-restore backup; existing copies retained",
            ));
        }
        datafs::room(&self.target, self.summary.peak_bytes)?;
        progress("backup");
        self.backup = Some(
            source.export(
                &backup_root.join("before.zip"),
                &self
                    .target
                    .join("jobs")
                    .join(format!("backup-{}", self.operation)),
                ExportOptions::default(),
                app_version,
                cancel,
                |_| {},
            )?,
        );
        let backup = self.backup.as_ref().unwrap();
        let mut open = fs::OpenOptions::new();
        open.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            open.share_mode(1);
        }
        self.backup_guard = Some(open.open(&backup.path)?);
        if datafs::hash(&backup.path, cancel)?.1 != backup.sha256 {
            return Err(invalid("pre-restore backup changed"));
        }
        datafs::json(
            &backup_root.join("backup.json"),
            &serde_json::json!({"format":1,"kind":"profile-restore","version":app_version,"created_utc":chrono::Utc::now(),"verified":true,"archive_sha256":self.backup.as_ref().unwrap().sha256}),
        )?;
        let publication = self
            .target
            .join("jobs")
            .join(format!("publication-{}", self.operation));
        let mut job = OwnedJob::new(&publication, "publication")?;
        job.preserve_if = Some(self.target.join("profile-restore.json"));
        self.publication = Some(job);
        for (i, file) in self.files.iter_mut().enumerate() {
            progress("prepare");
            let target = publication.join(i.to_string());
            if datafs::copy(
                &presets::inside(&self.inspected.root, &file.source)?,
                &target,
                cancel,
            )? != (file.bytes, file.sha256.clone())
            {
                return Err(invalid("restore asset changed"));
            }
            file.prepared = Some(target);
        }
        Ok(())
    }
    pub fn apply(self, store: &mut Sandbox, cancel: &AtomicBool) -> Result<ApplyReport> {
        datafs::check(cancel)?;
        if store.root != self.target
            || fingerprint(&store.root, &super::snapshot::current(store)?, cancel)?
                != self.fingerprint
        {
            return Err(invalid(
                "current profile changed; inspect the archive again",
            ));
        }
        if self.summary.already_imported {
            return Ok(ApplyReport {
                phrases_added: 0,
                presets_added: 0,
                backup: None,
                warning: None,
                already_imported: true,
            });
        }
        let backup = self
            .backup
            .as_ref()
            .ok_or_else(|| invalid("verified pre-restore backup is required"))?;
        let files = self
            .files
            .iter()
            .map(|f| {
                Ok(Publication {
                    entry: Entry {
                        path: f.target.clone(),
                        bytes: f.bytes,
                        sha256: f.sha256.clone(),
                    },
                    prepared: f
                        .prepared
                        .as_ref()
                        .ok_or_else(|| invalid("restore publication was not prepared"))?
                        .strip_prefix(&store.root)
                        .map_err(|_| invalid("publication must be on the target data volume"))?
                        .to_string_lossy()
                        .replace('\\', "/"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let journal = Journal {
            format: 1,
            archive: self.inspected.archive_sha256.clone(),
            files,
            dirs: {
                let mut dirs = std::collections::BTreeSet::new();
                for file in &self.files {
                    for dir in Path::new(&file.target).ancestors().skip(1) {
                        if dir.as_os_str().is_empty() {
                            break;
                        }
                        let name = dir.to_string_lossy().replace('\\', "/");
                        if !presets::inside(&store.root, &name)?.try_exists()? {
                            dirs.insert(name);
                        }
                    }
                }
                dirs.into_iter().collect()
            },
            backup: backup.path.clone(),
        };
        let journal_path = store.root.join("profile-restore.json");
        if journal_path.exists() {
            return Err(invalid(
                "unfinished restore must be recovered before another restore",
            ));
        }
        datafs::json(&journal_path, &journal)?;
        super::boundary("restore_journal");
        let result = (|| -> Result<()> {
            for file in &self.files {
                datafs::check(cancel)?;
                let from = file
                    .prepared
                    .as_ref()
                    .ok_or_else(|| invalid("restore files were not prepared"))?;
                let target = presets::inside(&store.root, &file.target)?;
                fs::create_dir_all(target.parent().unwrap())?;
                datafs::publish(from, &target)?;
                super::boundary("restore_file");
            }
            let tx = store.connection.transaction()?;
            for row in &self.rows {
                library::insert(&tx, row, true)?;
            }
            if self.summary.clean_profile {
                let mut prefs = self.inspected.snapshot.prefs.clone();
                prefs.desktop.recovery_limit_bytes = self.limit;
                for id in [
                    &mut prefs.desktop.character_preset,
                    &mut prefs.desktop.visual_preset,
                    &mut prefs.generation.voice_profile,
                ] {
                    if let Some(mapped) = self.presets.get(id) {
                        *id = mapped.clone();
                    }
                }
                tx.execute("INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&prefs)?])?;
                tx.execute("INSERT INTO reminder_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&self.inspected.snapshot.state)?])?;
                for (key, value) in &self.inspected.snapshot.meta {
                    if ["import_limit", "pending_recovery_limit"].contains(&key.as_str()) {
                        continue;
                    }
                    let key = match key.as_str() {
                        "origin" => "profile_origin_hint",
                        "recovery_path" => "profile_recovery_hint",
                        _ => key,
                    };
                    tx.execute(
                        "INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
                        rusqlite::params![key, serde_json::to_string(value)?],
                    )?;
                }
                for (notice, id) in &self.inspected.snapshot.claims {
                    tx.execute(
                        "INSERT INTO delivery_claims VALUES(?1,?2) ON CONFLICT(notice) DO NOTHING",
                        rusqlite::params![notice, id],
                    )?;
                }
            }
            if self.summary.clean_profile {
                tx.execute("INSERT INTO library_meta VALUES('legacy_semantics_v2','true') ON CONFLICT(key) DO UPDATE SET json='true'",[])?;
            } else {
                tx.execute("INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",rusqlite::params![format!("profile_history:{}",self.inspected.archive_sha256),serde_json::to_string(&serde_json::json!({"format":1,"snapshot":format!("legacy/profiles/{}/library.sqlite3",self.inspected.archive_sha256),"audio":self.audio_map,"presets":self.presets}))?])?;
            }
            tx.execute("INSERT INTO library_meta VALUES(?1,?2)",rusqlite::params![format!("profile_restore:{}",self.inspected.archive_sha256),serde_json::to_string(&serde_json::json!({"format":1,"created_utc":chrono::Utc::now(),"backup":backup.path,"phrases":self.rows.len()}))?])?;
            datafs::check(cancel)?;
            super::boundary("restore_before_commit");
            tx.commit()?;
            super::boundary("restore_after_commit");
            Ok(())
        })();
        if let Err(error) = result {
            recover(store)?;
            return Err(error);
        }
        let warning = recover(store)
            .err()
            .map(|e| format!("Restore committed; cleanup needs attention: {e}"));
        Ok(ApplyReport {
            phrases_added: self.rows.len(),
            presets_added: self.summary.presets_added,
            backup: Some(backup.path.clone()),
            warning,
            already_imported: false,
        })
    }
}
const MANIFEST_MARGIN: u64 = 4 * 1024 * 1024;
pub(crate) fn recover(store: &mut Sandbox) -> Result<()> {
    let path = store.root.join("profile-restore.json");
    crate::preview::no_link_chain(&path)?;
    if !path.try_exists()? {
        return Ok(());
    }
    if path.metadata()?.len() > 8 * 1024 * 1024 {
        return Err(invalid(
            "restore journal too large; retained for inspection",
        ));
    }
    let journal: Journal = serde_json::from_slice(&fs::read(&path)?)?;
    if journal.format != 1
        || journal.archive.len() != 64
        || !journal.archive.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("unknown restore journal; retained"));
    }
    let committed = store
        .meta(&format!("profile_restore:{}", journal.archive))?
        .is_some();
    for file in &journal.files {
        let entry = &file.entry;
        if !allowed(&entry.path) || entry.path == "library.sqlite3" {
            return Err(invalid("invalid restore cleanup path"));
        }
        if !file.prepared.starts_with("jobs/publication-") {
            return Err(invalid("invalid publication owner path"));
        }
        let prepared = presets::inside(&store.root, &file.prepared)?;
        let target = presets::inside(&store.root, &entry.path)?;
        if target.exists() {
            // A still-present source proves rename did not publish this file.
            // Never remove an identical file independently created by a user.
            if !committed && prepared.exists() {
                continue;
            }
            if datafs::hash(&target, &AtomicBool::new(false))?
                != (entry.bytes, entry.sha256.clone())
            {
                return Err(invalid(
                    "restored file was modified; automatic cleanup stopped",
                ));
            }
            if !committed {
                fs::remove_file(target)?;
                super::boundary("restore_cleanup_file");
            }
        } else if committed {
            return Err(invalid("committed restore has a missing file"));
        }
    }
    if !committed {
        let mut dirs = journal.dirs;
        dirs.sort_by_key(|s| std::cmp::Reverse(s.len()));
        dirs.dedup();
        for dir in dirs {
            let target = presets::inside(&store.root, &dir)?;
            if target.is_dir() && fs::read_dir(&target)?.next().is_none() {
                fs::remove_dir(target)?;
            }
        }
    }
    super::boundary("restore_cleanup_journal");
    fs::remove_file(path)?;
    Ok(())
}
