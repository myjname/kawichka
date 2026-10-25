//! Rust-owned working library. Original snapshots remain separate and immutable.
use crate::{
    Error, Result,
    guard::LegacyGuard,
    preview::{self, Phase},
    sandbox::Sandbox,
};
use kavychka_core::{engine::State, legacy::SettingsSnapshot, preferences::Preferences};
use rusqlite::{OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phrase {
    pub id: String,
    pub text: String,
    pub category: String,
    pub model: String,
    pub profile: String,
    pub day: Option<String>,
    pub audio_path: Option<String>,
    pub approved: bool,
    pub deleted: bool,
    pub rejected: bool,
    pub used_at: Option<String>,
    pub created_at: String,
    pub followup: bool,
    pub language: String,
    #[serde(default = "legacy_seed")]
    pub seed: i64,
    #[serde(default)]
    pub voice_style: String,
    #[serde(default)]
    pub voice_instruction: String,
    #[serde(default)]
    pub voice_reference: String,
}
fn legacy_seed() -> i64 {
    42
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Origin {
    pub source: PathBuf,
    pub snapshot: PathBuf,
    pub imported_rows: usize,
}
pub(crate) const LIBRARY_SQL: &str = "BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS phrases(id TEXT PRIMARY KEY,text TEXT NOT NULL,normalized TEXT NOT NULL,category TEXT NOT NULL,model TEXT NOT NULL,profile TEXT NOT NULL,day TEXT,audio_path TEXT,approved INTEGER NOT NULL DEFAULT 0,deleted INTEGER NOT NULL DEFAULT 0,rejected INTEGER NOT NULL DEFAULT 0,used_at TEXT,created_at TEXT NOT NULL,followup INTEGER NOT NULL DEFAULT 0,language TEXT NOT NULL DEFAULT 'ru',seed INTEGER NOT NULL DEFAULT 42,voice_style TEXT NOT NULL DEFAULT '',voice_instruction TEXT NOT NULL DEFAULT '',voice_reference TEXT NOT NULL DEFAULT '');
            CREATE INDEX IF NOT EXISTS phrases_queue ON phrases(day,category,approved,used_at,deleted,rejected,language,followup);
            CREATE TABLE IF NOT EXISTS library_meta(key TEXT PRIMARY KEY,json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS delivery_claims(notice TEXT PRIMARY KEY,phrase_id TEXT NOT NULL);
            PRAGMA user_version=3; COMMIT;";
pub(crate) fn read_phrases(connection: &rusqlite::Connection) -> Result<Vec<Phrase>> {
    query_phrases(connection, "ORDER BY created_at DESC,id", [])
}
fn query_phrases(
    connection: &rusqlite::Connection,
    suffix: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Phrase>> {
    let mut q=connection.prepare(&format!("SELECT id,text,category,model,profile,day,audio_path,approved,deleted,rejected,used_at,created_at,followup,language,seed,voice_style,voice_instruction,voice_reference FROM phrases {suffix}"))?;
    let rows = q.query_map(params, |r| {
        Ok(Phrase {
            id: r.get(0)?,
            text: r.get(1)?,
            category: r.get(2)?,
            model: r.get(3)?,
            profile: r.get(4)?,
            day: r.get(5)?,
            audio_path: r.get(6)?,
            approved: r.get(7)?,
            deleted: r.get(8)?,
            rejected: r.get(9)?,
            used_at: r.get(10)?,
            created_at: r.get(11)?,
            followup: r.get(12)?,
            language: r.get(13)?,
            seed: r.get(14)?,
            voice_style: r.get(15)?,
            voice_instruction: r.get(16)?,
            voice_reference: r.get(17)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)
}
impl Sandbox {
    pub fn set_recovery_limit(&mut self, bytes: u64) -> Result<()> {
        let mut prefs = self.load()?;
        prefs.desktop.recovery_limit_bytes = bytes;
        prefs.validate().map_err(Error::Invalid)?;
        let exists: bool =
            self.connection
                .query_row("SELECT EXISTS(SELECT 1 FROM preferences)", [], |r| r.get(0))?;
        let tx = self.connection.transaction()?;
        tx.execute("INSERT INTO library_meta VALUES('import_limit',?1) ON CONFLICT(key) DO UPDATE SET json=excluded.json",[bytes.to_string()])?;
        if exists {
            tx.execute(
                "UPDATE preferences SET json=?1 WHERE id=1",
                [serde_json::to_string(&prefs)?],
            )?;
        } else {
            tx.execute("INSERT INTO library_meta VALUES('pending_recovery_limit',?1) ON CONFLICT(key) DO UPDATE SET json=excluded.json",[bytes.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn apply_pending_recovery_limit(&mut self) -> Result<()> {
        if !self.has_preferences()? {
            return Ok(());
        }
        if let Some(value) = self.meta("pending_recovery_limit")? {
            let bytes = value
                .as_u64()
                .ok_or_else(|| Error::Invalid("invalid recovery limit".into()))?;
            let mut prefs = self.load()?;
            prefs.desktop.recovery_limit_bytes = bytes;
            prefs.validate().map_err(Error::Invalid)?;
            let tx = self.connection.transaction()?;
            tx.execute("INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&prefs)?])?;
            tx.execute(
                "DELETE FROM library_meta WHERE key='pending_recovery_limit'",
                [],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
    pub(crate) fn initialize_library(&mut self) -> Result<()> {
        let version: u32 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 3 {
            self.connection.execute_batch(LIBRARY_SQL)?;
        }
        for name in ["audio", "presets", "jobs", "exports"] {
            let path = self.root.join(name);
            preview::no_link_chain(&path)?;
            std::fs::create_dir_all(path)?;
        }
        Ok(())
    }
    pub fn origin(&self) -> Result<Option<Origin>> {
        self.meta("origin")?
            .map(|v| serde_json::from_value(v).map_err(Error::from))
            .transpose()
    }
    pub fn meta(&self, key: &str) -> Result<Option<Value>> {
        let s: Option<String> = self
            .connection
            .query_row("SELECT json FROM library_meta WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        s.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn put_meta(&self, key: &str, value: &Value) -> Result<()> {
        self.connection.execute("INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET json=excluded.json",params![key,serde_json::to_string(value)?])?;
        Ok(())
    }
    /// Upgrade the semantics of an earlier preview once, without replaying an old
    /// checkpoint over a series already started by this version.
    pub fn complete_legacy_semantics(
        &mut self,
        clock: kavychka_core::engine::Clock,
        resolve: impl FnMut(
            chrono::NaiveDate,
            u16,
        ) -> std::result::Result<chrono::DateTime<chrono::Utc>, String>,
    ) -> Result<()> {
        if self.meta("legacy_semantics_v2")?.is_some() {
            return Ok(());
        }
        let Some(raw) = self.meta("legacy_settings")? else {
            return Ok(());
        };
        let settings: SettingsSnapshot = serde_json::from_value(raw)?;
        let mut prefs = self.load()?;
        let recovery_limit = prefs.desktop.recovery_limit_bytes;
        if self.meta("legacy_keep_current_preferences")? != Some(json!(true)) {
            prefs.desktop = kavychka_core::legacy_active::preferences(&settings)
                .map_err(Error::Invalid)?
                .desktop;
        }
        prefs.desktop.recovery_limit_bytes = recovery_limit;
        let checkpoint = if let Some(origin) = self.origin()? {
            let path = origin.snapshot.join("reminder-state.json");
            preview::no_link_chain(&path)?;
            if path.try_exists()? {
                let b = std::fs::read(path)?;
                if b.len() > 32768 {
                    return Err(Error::Invalid("checkpoint too large".into()));
                }
                serde_json::from_slice(b.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&b))?
            } else {
                Value::Null
            }
        } else {
            Value::Null
        };
        let legacy =
            kavychka_core::legacy_state::convert(&settings, &checkpoint, &prefs, clock, resolve)
                .map_err(Error::Invalid)?;
        let mut state = self.load_state()?;
        let has_runtime: bool =
            self.connection
                .query_row("SELECT EXISTS(SELECT 1 FROM reminder_state)", [], |r| {
                    r.get(0)
                })?;
        if !has_runtime {
            state = legacy
        } else {
            for (id, t) in legacy.handled {
                state.handled.entry(id).or_insert(t);
            }
        }
        if !prefs.desktop.character_preset.is_empty() {
            let p = crate::presets::read(&self.root, &prefs.desktop.character_preset)?;
            if let crate::presets::Content::Character { description } = p.content {
                prefs.generation.character_description = description
            } else {
                return Err(Error::Invalid(
                    "selected character preset has another kind".into(),
                ));
            }
        }
        if !prefs.desktop.visual_preset.is_empty() {
            let p = crate::presets::read(&self.root, &prefs.desktop.visual_preset)?;
            if !matches!(p.content, crate::presets::Content::Visual { .. }) {
                return Err(Error::Invalid(
                    "selected visual preset has another kind".into(),
                ));
            }
        }
        if !prefs.generation.voice_profile.is_empty() {
            let p = crate::presets::read(&self.root, &prefs.generation.voice_profile)?;
            if !matches!(p.content, crate::presets::Content::Voice { .. }) {
                return Err(Error::Invalid(
                    "selected voice preset has another kind".into(),
                ));
            }
        }
        prefs.validate().map_err(Error::Invalid)?;
        state.validate().map_err(Error::Invalid)?;
        let mut preparation = Vec::<Value>::new();
        if let Some(origin) = self.origin()? {
            let db = rusqlite::Connection::open_with_flags(
                origin.snapshot.join("library.snapshot.sqlite3"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='preparation')",[],|r|r.get(0))?;
            if exists {
                let mut q=db.prepare("SELECT day,MAX(attempts),MAX(retry_at) FROM preparation WHERE state<>'ready' GROUP BY day")?;
                let rows = q.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, f64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (day, attempts, retry) = row?;
                    let next = chrono::DateTime::from_timestamp_millis((retry * 1000.0) as i64);
                    preparation.push(json!({"day":day,"attempts":attempts.max(0),"next":next}));
                }
            }
        }
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(&prefs)?],
        )?;
        tx.execute("INSERT INTO reminder_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(&state)?])?;
        tx.execute(
            "INSERT INTO library_meta VALUES('legacy_semantics_v2','true')",
            [],
        )?;
        tx.execute("INSERT INTO library_meta VALUES('legacy_preparation_states',?1) ON CONFLICT(key) DO NOTHING",[serde_json::to_string(&preparation)?])?;
        tx.commit()?;
        Ok(())
    }
    /// Once per new working store. A changed source never silently reimports.
    pub fn bootstrap_legacy(&mut self, source: &Path, cancel: &AtomicBool) -> Result<Origin> {
        if let Some(origin) = self.origin()? {
            let supplied = source.canonicalize().unwrap_or_else(|_| {
                std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf())
            });
            if !crate::transfer::owned_tail(&origin.source, &supplied)
                .is_some_and(|tail| tail.as_os_str().is_empty())
            {
                return Err(Error::Invalid(
                    "this preview belongs to another source".into(),
                ));
            }
            return Ok(origin);
        }
        let snapshot = self.root.join("legacy");
        let guard = LegacyGuard::acquire()?;
        let limit = self
            .meta("import_limit")?
            .and_then(|v| v.as_u64())
            .unwrap_or(2_000_000_000);
        let report = preview::prepare(source, &snapshot, limit, cancel, &guard, |_| {})?;
        if report.phase != Phase::Ready {
            return Err(Error::Invalid("import is not ready".into()));
        }
        self.import_snapshot(source, &snapshot)
    }
    pub fn import_snapshot(&mut self, source: &Path, snapshot: &Path) -> Result<Origin> {
        preview::no_link_chain(snapshot)?;
        let report: preview::Report =
            serde_json::from_slice(&std::fs::read(snapshot.join("preview-report.json"))?)?;
        if report.format != 1
            || report.phase != Phase::Ready
            || report.source.canonicalize()? != source.canonicalize()?
        {
            return Err(Error::Invalid("unverified or unrelated snapshot".into()));
        }
        let db = snapshot.join("library.snapshot.sqlite3");
        preview::no_link_chain(&db)?;
        let old = rusqlite::Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let settings: SettingsSnapshot = serde_json::from_str(&old.query_row(
            "SELECT json FROM app_settings WHERE id=1",
            [],
            |r| r.get::<_, String>(0),
        )?)?;
        let mut effective = settings.clone();
        #[cfg(windows)]
        {
            let language =
                if unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff
                    == 0x19
                {
                    "ru"
                } else {
                    "en"
                };
            for key in ["UiLanguage", "SpeechLanguage"] {
                effective
                    .desktop
                    .entry(key.into())
                    .or_insert(json!(language));
            }
        }
        let mut preferences =
            kavychka_core::legacy_active::preferences(&effective).map_err(Error::Invalid)?;
        for (id, path) in &report.model_roots {
            if kavychka_core::generation::TEXT_MODELS.contains(&id.as_str())
                || kavychka_core::generation::VOICE_MODELS.contains(&id.as_str())
            {
                preferences
                    .generation
                    .model_paths
                    .insert(id.clone(), path.clone());
            }
        }
        if let Some(limit) = self.meta("import_limit")?.and_then(|v| v.as_u64()) {
            preferences.desktop.recovery_limit_bytes = limit;
        }
        preferences.validate().map_err(Error::Invalid)?;
        let mut rows = Vec::new();
        let exists: i64 = old.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='phrases'",
            [],
            |r| r.get(0),
        )?;
        if exists > 0 {
            let mut q = old.prepare("SELECT * FROM phrases ORDER BY rowid")?;
            let names: Vec<String> = q.column_names().iter().map(|v| v.to_string()).collect();
            let mut iter = q.query([])?;
            while let Some(row) = iter.next()? {
                let mut v = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    use rusqlite::types::ValueRef;
                    let value = match row.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => json!(n),
                        ValueRef::Real(n) => json!(n),
                        ValueRef::Text(s) => json!(
                            std::str::from_utf8(s)
                                .map_err(|_| Error::Invalid("non UTF-8 phrase".into()))?
                        ),
                        ValueRef::Blob(_) => {
                            return Err(Error::Invalid("unexpected binary phrase column".into()));
                        }
                    };
                    v.insert(name.clone(), value);
                }
                for name in ["approved", "deleted", "rejected", "followup"] {
                    let value = v.get(name).and_then(Value::as_i64).unwrap_or(0);
                    v.insert(name.into(), json!(value != 0));
                }
                v.entry("language").or_insert(json!("ru"));
                let phrase: Phrase = serde_json::from_value(Value::Object(v))?;
                validate_phrase(&phrase)?;
                rows.push(phrase);
            }
        }
        let imported_rows = rows.len();
        merge_builtin_history(snapshot, &settings, &mut rows)?;
        // Files first, transaction last. An interrupted copy can be safely retried.
        for dir in ["audio", "presets", "builtin-days"] {
            let path = snapshot.join(dir);
            if path.exists() {
                copy_tree(&path, &self.root.join(dir))?;
            }
        }
        let has_preferences: bool =
            self.connection
                .query_row("SELECT EXISTS(SELECT 1 FROM preferences)", [], |r| r.get(0))?;
        let origin = Origin {
            source: source.canonicalize()?,
            snapshot: snapshot.canonicalize()?,
            imported_rows,
        };
        let tx = self.connection.transaction()?;
        for row in &rows {
            insert(&tx, row, true)?;
        }
        if !has_preferences {
            tx.execute(
                "INSERT INTO preferences VALUES(1,?1)",
                [serde_json::to_string(&preferences)?],
            )?;
        } else {
            // The one-time semantic upgrade must not undo choices already made
            // in the destination. Older previews without this marker still get
            // their missing desktop fields upgraded once.
            tx.execute("INSERT INTO library_meta VALUES('legacy_keep_current_preferences','true') ON CONFLICT(key) DO NOTHING", [])?;
        }
        tx.execute(
            "INSERT INTO library_meta VALUES('legacy_settings',?1) ON CONFLICT(key) DO NOTHING",
            [serde_json::to_string(&settings)?],
        )?;
        tx.execute(
            "INSERT INTO library_meta VALUES('origin',?1) ON CONFLICT(key) DO NOTHING",
            [serde_json::to_string(&origin)?],
        )?;
        let has_rotation:bool=old.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='voice_rotation')",[],|r|r.get(0))?;
        if has_rotation {
            let rotation: Option<i64> = old
                .query_row(
                    "SELECT next_index FROM voice_rotation WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(n) = rotation {
                tx.execute("INSERT INTO library_meta VALUES('voice_rotation',?1) ON CONFLICT(key) DO NOTHING",[n.max(0).to_string()])?;
            }
        }
        tx.commit()?;
        Ok(origin)
    }
    pub fn phrases(&self) -> Result<Vec<Phrase>> {
        read_phrases(&self.connection)
    }
    pub fn phrase(&self, id: &str) -> Result<Phrase> {
        query_phrases(&self.connection, "WHERE id=?1", [id])?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Invalid("phrase not found".into()))
    }
    pub fn phrase_page(
        &self,
        day: Option<&str>,
        page: usize,
    ) -> Result<(Vec<Phrase>, usize, usize)> {
        const PAGE_SIZE: usize = 50;
        let total = self.connection.query_row(
            "SELECT count(*) FROM phrases WHERE deleted=0 AND (?1 IS NULL OR day=?1)",
            [day],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let page = page.min(total.saturating_sub(1) / PAGE_SIZE);
        let rows = query_phrases(
            &self.connection,
            "WHERE deleted=0 AND (?1 IS NULL OR day=?1) ORDER BY created_at DESC,id LIMIT ?2 OFFSET ?3",
            params![day, PAGE_SIZE as i64, (page * PAGE_SIZE) as i64],
        )?;
        Ok((rows, total, page))
    }
    pub fn has_voice_reference(&self, id: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM phrases WHERE voice_reference=?1)",
            [id],
            |r| r.get(0),
        )?)
    }
    /// A stopped child owns only this flat audio staging folder. Logs are retained.
    pub fn cleanup_staging(&self, job: &Path) -> Result<()> {
        preview::no_link_chain(job)?;
        if job
            .parent()
            .ok_or_else(|| Error::Invalid("invalid job path".into()))?
            .canonicalize()?
            != self.root.join("jobs").canonicalize()?
            || !job
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("job-"))
        {
            return Err(Error::Invalid("job cleanup escaped owned jobs".into()));
        }
        let audio = job.join("audio");
        preview::no_link_chain(&audio)?;
        if audio.is_dir() {
            for file in std::fs::read_dir(&audio)? {
                let file = file?;
                preview::no_link_chain(&file.path())?;
                if !file.file_type()?.is_file() {
                    return Err(Error::Invalid("unexpected staging directory".into()));
                }
                std::fs::remove_file(file.path())?;
            }
            std::fs::remove_dir(audio)?;
        }
        for file in std::fs::read_dir(job)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().to_string();
            if name.starts_with("llama-") && name.ends_with(".key") {
                preview::no_link_chain(&file.path())?;
                std::fs::remove_file(file.path())?;
            }
        }
        for name in ["payload.json", "library.zip", "cancel.request"] {
            let file = job.join(name);
            preview::no_link_chain(&file)?;
            if file.is_file() {
                std::fs::remove_file(file)?;
            }
        }
        let presets = job.join("presets");
        if presets.is_dir() {
            checked_remove_tree(&presets, job)?;
        }
        for name in ["hf", "torch", "numba"] {
            let cache = job.join(name);
            preview::no_link_chain(&cache)?;
            if cache.is_dir() {
                checked_remove_tree(&cache, job)?;
            }
        }
        Ok(())
    }
    pub fn prune_quarantine(&self, today: chrono::NaiveDate) -> Result<usize> {
        let root = self.root.join("quarantine");
        preview::no_link_chain(&root)?;
        if !root.try_exists()? {
            return Ok(0);
        }
        let mut count = 0;
        for item in std::fs::read_dir(&root)? {
            let item = item?;
            let day = item
                .file_name()
                .to_string_lossy()
                .parse::<chrono::NaiveDate>();
            if day.is_ok_and(|day| today - day > chrono::Duration::days(7))
                && item.file_type()?.is_dir()
            {
                count += 1;
                checked_remove_tree(&item.path(), &root)?;
            }
        }
        Ok(count)
    }
    pub fn choose(
        &self,
        day: &str,
        category: &str,
        followup: bool,
        language: &str,
    ) -> Result<Option<Phrase>> {
        Ok(query_phrases(&self.connection,
            "WHERE day=?1 AND category=?2 AND followup=?3 AND language=?4 AND approved=1 AND deleted=0 AND rejected=0 AND used_at IS NULL AND (audio_path IS NOT NULL OR model='system') ORDER BY created_at ASC,id DESC LIMIT 1",
            params![day,category,followup,language])?.into_iter().next())
    }
    pub fn select_phrase(&self, id: &str, keep: bool) -> Result<()> {
        if self.connection.execute(
            "UPDATE phrases SET approved=?2,rejected=?3 WHERE id=?1 AND deleted=0",
            params![id, keep, !keep],
        )? != 1
        {
            return Err(Error::Invalid("phrase not found".into()));
        }
        Ok(())
    }
    pub fn audio_file(&self, row: &Phrase) -> Result<Option<PathBuf>> {
        row.audio_path
            .as_ref()
            .map(|name| {
                safe_name(name)?;
                let path = self.root.join("audio").join(name);
                preview::no_link_chain(&path)?;
                if !path.is_file() {
                    return Err(Error::Invalid("recording is missing".into()));
                }
                Ok(path)
            })
            .transpose()
    }
    pub fn accept_phrase(&mut self, row: &Phrase, staging: Option<&Path>) -> Result<()> {
        self.accept_phrase_and_rotation(row, staging, None, None)
    }
    pub fn accept_phrase_and_rotation(
        &mut self,
        row: &Phrase,
        staging: Option<&Path>,
        rotation: Option<u64>,
        purpose: Option<&str>,
    ) -> Result<()> {
        validate_phrase(row)?;
        if let Some(name) = &row.audio_path {
            let from = staging
                .ok_or_else(|| Error::Invalid("missing audio staging".into()))?
                .join("audio")
                .join(name);
            preview::no_link_chain(&from)?;
            copy_verified(&from, &self.root.join("audio").join(name))?;
        }
        let tx = self.connection.transaction()?;
        insert(&tx, row, false)?;
        if let Some(purpose) = purpose {
            tx.execute(
                "INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
                params![
                    format!("purpose:{}", row.id),
                    serde_json::to_string(purpose)?
                ],
            )?;
        }
        if row.approved {
            tx.execute(
                "UPDATE phrases SET approved=1 WHERE id=?1 AND rejected=0 AND deleted=0",
                [&row.id],
            )?;
        }
        if let Some(rotation) = rotation {
            tx.execute("INSERT INTO library_meta VALUES('voice_rotation',?1) ON CONFLICT(key) DO UPDATE SET json=excluded.json",[rotation.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_delivery(
        &mut self,
        prefs: &Preferences,
        state: &State,
        claim: Option<(&str, &str)>,
    ) -> Result<()> {
        self.save_delivery_checkpoint(prefs, state, claim, None, None)
    }
    pub fn save_delivery_checkpoint(
        &mut self,
        prefs: &Preferences,
        state: &State,
        claim: Option<(&str, &str)>,
        presenting: Option<&kavychka_core::engine::Notice>,
        confirmed: Option<&str>,
    ) -> Result<()> {
        self.save_delivery_archived(prefs, state, claim, presenting, confirmed, &[])
    }
    pub fn save_delivery_archived(
        &mut self,
        prefs: &Preferences,
        state: &State,
        claim: Option<(&str, &str)>,
        presenting: Option<&kavychka_core::engine::Notice>,
        confirmed: Option<&str>,
        archived: &[kavychka_core::engine::Notice],
    ) -> Result<()> {
        prefs.validate().map_err(Error::Invalid)?;
        state.validate().map_err(Error::Invalid)?;
        let tx = self.connection.transaction()?;
        for notice in archived {
            if confirmed != Some(notice.key.as_str()) {
                crate::delivery::archive_notice(&tx, notice)?;
            }
        }
        if let Some((notice, id)) = claim {
            if tx.execute("UPDATE phrases SET used_at=?2 WHERE id=?1 AND used_at IS NULL AND approved=1 AND deleted=0 AND rejected=0",params![id,chrono_now()])?!=1{return Err(Error::Invalid("phrase already consumed or excluded".into()));}
            tx.execute(
                "INSERT INTO delivery_claims VALUES(?1,?2)",
                params![notice, id],
            )?;
            crate::delivery::write_phase(&tx, notice, "committed", &[], Some(id))?;
        }
        if let Some(notice) = presenting {
            crate::delivery::write_phase(
                &tx,
                &notice.key,
                "committed",
                &notice
                    .events
                    .iter()
                    .map(|e| e.id.clone())
                    .collect::<Vec<_>>(),
                claim.map(|(_, id)| id),
            )?;
        }
        if let Some(notice) = confirmed {
            crate::delivery::write_phase(&tx, notice, "confirmed", &[], None)?;
            tx.execute(
                "DELETE FROM library_meta WHERE key=?1",
                [format!("pending_notice:{notice}")],
            )?;
        }
        tx.execute(
            "INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [serde_json::to_string(prefs)?],
        )?;
        tx.execute("INSERT INTO reminder_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[serde_json::to_string(state)?])?;
        tx.commit()?;
        Ok(())
    }
    /// Text is permanent. Audio is detached into a reversible quarantine first.
    pub fn expire_audio(&mut self, day: &str, all: bool) -> Result<usize> {
        chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| Error::Invalid("invalid cleanup day".into()))?;
        let rows = self.phrases()?;
        let expire: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.audio_path.is_some() && (all || r.day.as_deref().is_some_and(|d| d < day))
            })
            .collect();
        let quarantine = self.root.join("quarantine").join(day);
        preview::no_link_chain(&quarantine)?;
        std::fs::create_dir_all(&quarantine)?;
        let referenced = rows
            .iter()
            .filter_map(|r| r.audio_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut paths = std::collections::BTreeMap::<String, PathBuf>::new();
        for row in &expire {
            let name = row.audio_path.as_ref().unwrap();
            safe_name(name)?;
            let path = self.root.join("audio").join(name);
            preview::no_link_chain(&path)?;
            if path.is_file() {
                paths.insert(name.clone(), path);
            }
        }
        for file in std::fs::read_dir(self.root.join("audio"))? {
            let file = file?;
            preview::no_link_chain(&file.path())?;
            let name = file.file_name().to_string_lossy().to_string();
            if !file.file_type()?.is_file() {
                return Err(Error::Invalid("unexpected directory in recordings".into()));
            }
            if !referenced.contains(&name) {
                paths.insert(name, file.path());
            }
        }
        let required: u64 = paths
            .iter()
            .filter(|(name, _)| !quarantine.join(name).exists())
            .map(|(_, path)| path.metadata().map(|m| m.len()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .sum();
        let reserves = if let Some(Value::String(path)) = self.meta("recovery_path")? {
            if !Path::new(&path).is_dir() {
                return Err(Error::Invalid(
                    "recovery storage is unavailable; recordings retained".into(),
                ));
            }
            directory_bytes(Path::new(&path))?
        } else {
            0
        };
        if directory_bytes(&self.root.join("quarantine"))?
            .saturating_add(required)
            .saturating_add(reserves)
            .saturating_add(directory_bytes(&self.root.join("recovery"))?)
            > self.load()?.desktop.recovery_limit_bytes
        {
            return Err(Error::Invalid(
                "recovery/quarantine limit exceeded; recordings and recovery copies preserved. Increase the limit or free space".into(),
            ));
        }
        for (name, from) in &paths {
            copy_verified(from, &quarantine.join(name))?;
        }
        let tx = self.connection.transaction()?;
        for r in &expire {
            tx.execute("UPDATE phrases SET audio_path=NULL WHERE id=?1", [&r.id])?;
        }
        tx.commit()?;
        for (name, path) in &paths {
            let still_used: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM phrases WHERE audio_path=?1)",
                [name],
                |r| r.get(0),
            )?;
            if !still_used {
                preview::no_link_chain(path)?;
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        Ok(expire.len())
    }
}
fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn checked_remove_tree(path: &Path, owner: &Path) -> Result<()> {
    preview::no_link_chain(path)?;
    let path = path.canonicalize()?;
    let owner = owner.canonicalize()?;
    if path == owner || !path.starts_with(&owner) {
        return Err(Error::Invalid("cleanup escaped owner".into()));
    }
    for f in std::fs::read_dir(&path)? {
        let f = f?;
        preview::no_link_chain(&f.path())?;
        if f.file_type()?.is_dir() {
            checked_remove_tree(&f.path(), &path)?
        } else {
            std::fs::remove_file(f.path())?;
        }
    }
    std::fs::remove_dir(path)?;
    Ok(())
}
fn merge_builtin_history(
    snapshot: &Path,
    settings: &SettingsSnapshot,
    rows: &mut Vec<Phrase>,
) -> Result<()> {
    let mut ids: std::collections::BTreeSet<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut texts: std::collections::BTreeSet<String> =
        rows.iter().map(|r| r.text.clone()).collect();
    let mut trimmed: std::collections::BTreeSet<String> =
        rows.iter().map(|r| r.text.trim().to_owned()).collect();
    let history = snapshot.join("builtin-days");
    if history.is_dir() {
        for file in std::fs::read_dir(history)? {
            let file = file?;
            if file.path().extension().is_none_or(|e| e != "json") {
                continue;
            }
            preview::no_link_chain(&file.path())?;
            if file.metadata()?.len() > 32_000_000 {
                return Err(Error::Invalid("built-in history exceeds limit".into()));
            }
            let bytes = std::fs::read(file.path())?;
            let day: Value =
                serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
            if day["Version"] != 1
                || !["ru", "en"].contains(&day["Language"].as_str().unwrap_or(""))
            {
                return Err(Error::Invalid("unsupported built-in history".into()));
            }
            let entries = day["Rows"]
                .as_array()
                .ok_or_else(|| Error::Invalid("invalid built-in rows".into()))?;
            if entries.len() > 50000 {
                return Err(Error::Invalid("built-in row limit exceeded".into()));
            }
            for row in entries {
                let id = row["Id"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("missing built-in id".into()))?;
                let text = row["Text"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("missing built-in text".into()))?;
                if ids.contains(id) || row["Kind"] == "external" && trimmed.contains(text.trim()) {
                    continue;
                }
                let phrase = Phrase {
                    id: id.into(),
                    text: text.into(),
                    category: row["Category"].as_str().unwrap_or("").into(),
                    model: "builtin".into(),
                    profile: "builtin-v1".into(),
                    day: day["Day"].as_str().map(str::to_owned),
                    audio_path: None,
                    approved: false,
                    deleted: true,
                    rejected: false,
                    used_at: row["UsedAt"].as_str().map(str::to_owned),
                    created_at: row["CreatedAt"]
                        .as_str()
                        .unwrap_or("1970-01-01T00:00:00Z")
                        .into(),
                    followup: row["Followup"]
                        .as_bool()
                        .ok_or_else(|| Error::Invalid("invalid built-in followup".into()))?,
                    language: day["Language"].as_str().unwrap().into(),
                    seed: 42,
                    voice_style: String::new(),
                    voice_instruction: String::new(),
                    voice_reference: String::new(),
                };
                validate_phrase(&phrase)?;
                ids.insert(phrase.id.clone());
                texts.insert(phrase.text.clone());
                trimmed.insert(phrase.text.trim().to_owned());
                rows.push(phrase);
            }
        }
    }
    if let Some(history) = settings.desktop.get("History").and_then(Value::as_array) {
        for text in history {
            let text = text
                .as_str()
                .ok_or_else(|| Error::Invalid("invalid legacy text history".into()))?;
            if text.trim().is_empty() || !texts.insert(text.to_owned()) {
                continue;
            }
            let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
            let phrase = Phrase {
                id: format!("history-{}", &hash[..32]),
                text: text.into(),
                category: "break".into(),
                model: "builtin".into(),
                profile: "legacy-history".into(),
                day: None,
                audio_path: None,
                approved: false,
                deleted: true,
                rejected: false,
                used_at: None,
                created_at: "1970-01-01T00:00:00Z".into(),
                followup: false,
                language: settings
                    .desktop
                    .get("SpeechLanguage")
                    .and_then(Value::as_str)
                    .unwrap_or("ru")
                    .into(),
                seed: 42,
                voice_style: String::new(),
                voice_instruction: String::new(),
                voice_reference: String::new(),
            };
            validate_phrase(&phrase)?;
            rows.push(phrase);
        }
    }
    Ok(())
}
pub fn safe_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 200
        || name.contains(['/', '\\', ':'])
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
        || crate::presets::is_reserved_component(name)
    {
        return Err(Error::Invalid("invalid library filename".into()));
    }
    Ok(())
}
pub fn validate_phrase(p: &Phrase) -> Result<()> {
    safe_name(&p.id)?;
    if !kavychka_core::generation::CATEGORIES.contains(&p.category.as_str())
        || !["ru", "en"].contains(&p.language.as_str())
        || p.text.trim().is_empty()
        || p.text.chars().count() > 4000
    {
        return Err(Error::Invalid("invalid phrase result".into()));
    }
    if let Some(n) = &p.audio_path {
        safe_name(n)?;
    }
    Ok(())
}
pub(crate) fn insert(tx: &rusqlite::Transaction<'_>, p: &Phrase, ignore: bool) -> Result<()> {
    let sql = if ignore {
        "INSERT OR IGNORE INTO phrases VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)"
    } else {
        "INSERT INTO phrases VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19) ON CONFLICT(id) DO UPDATE SET model=excluded.model,profile=excluded.profile,audio_path=excluded.audio_path,seed=excluded.seed,voice_style=excluded.voice_style,voice_instruction=excluded.voice_instruction,voice_reference=excluded.voice_reference"
    };
    tx.execute(
        sql,
        params![
            p.id,
            p.text,
            p.text.to_lowercase(),
            p.category,
            p.model,
            p.profile,
            p.day,
            p.audio_path,
            p.approved,
            p.deleted,
            p.rejected,
            p.used_at,
            p.created_at,
            p.followup,
            p.language,
            p.seed,
            p.voice_style,
            p.voice_instruction,
            p.voice_reference
        ],
    )?;
    Ok(())
}
fn copy_verified(from: &Path, to: &Path) -> Result<()> {
    preview::no_link_chain(from)?;
    preview::no_link_chain(to)?;
    let cancel = AtomicBool::new(false);
    if to.exists() {
        if crate::datafs::hash(from, &cancel)? != crate::datafs::hash(to, &cancel)? {
            return Err(Error::Invalid(
                "asset name collision; existing file preserved".into(),
            ));
        }
    } else {
        // A deterministic .copying name used to truncate and rename an
        // unrelated neighbor. Only own a new, unique temporary file, and stream
        // large legacy assets rather than allocating each file twice in memory.
        let temporary = to.with_file_name(format!("{}.copying", crate::datafs::id()));
        preview::no_link_chain(&temporary)?;
        let mut source = std::fs::File::open(from)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            std::io::copy(&mut source, &mut file)?;
            file.sync_all()?;
            drop(file);
            if crate::datafs::hash(&temporary, &cancel)? != crate::datafs::hash(from, &cancel)? {
                return Err(Error::SourceChanged);
            }
            crate::datafs::publish(&temporary, to)
        })();
        if temporary.try_exists()? {
            std::fs::remove_file(&temporary)?;
        }
        result?;
    }
    Ok(())
}
fn directory_bytes(root: &Path) -> Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    preview::no_link_chain(root)?;
    let mut total = 0;
    for item in std::fs::read_dir(root)? {
        let item = item?;
        preview::no_link_chain(&item.path())?;
        total += if item.file_type()?.is_dir() {
            directory_bytes(&item.path())?
        } else {
            item.metadata()?.len()
        };
    }
    Ok(total)
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    preview::no_link_chain(from)?;
    preview::no_link_chain(to)?;
    std::fs::create_dir_all(to)?;
    for item in std::fs::read_dir(from)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            copy_tree(&item.path(), &to.join(item.file_name()))?;
        } else {
            copy_verified(&item.path(), &to.join(item.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copying_asset_never_truncates_a_neighbor_with_copying_extension() {
        let dir = temp();
        let source = dir.path().join("source.wav");
        let target = dir.path().join("target.wav");
        let neighbor = target.with_extension("copying");
        std::fs::write(&source, vec![42; 2_000_000]).unwrap();
        std::fs::write(&neighbor, b"unrelated file must survive").unwrap();
        copy_verified(&source, &target).unwrap();
        assert_eq!(
            std::fs::read(&neighbor).unwrap(),
            b"unrelated file must survive"
        );
        assert_eq!(
            crate::datafs::hash(&source, &AtomicBool::new(false)).unwrap(),
            crate::datafs::hash(&target, &AtomicBool::new(false)).unwrap()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
    #[test]
    fn library_filenames_reject_windows_aliases_and_control_characters() {
        for name in [
            "recording.wav.",
            "recording.wav ",
            "NUL.flac",
            "COM1.wav",
            "clip\u{0}.flac",
            "a?.wav",
        ] {
            assert!(
                safe_name(name).is_err(),
                "accepted unsafe library filename {name:?}"
            );
        }
        for name in ["normal.wav", "длинная запись.flac", "row-1"] {
            safe_name(name).unwrap();
        }
    }
    #[test]
    fn large_legacy_history_preserves_deduplication_and_validates_rows() {
        let dir = temp();
        let mut rows = vec![phrase()];
        let original = rows[0].text.clone();
        let history = (0..10_000)
            .map(|i| format!("Прежняя фраза {i}"))
            .chain([original.clone(), original]);
        let settings = kavychka_core::legacy::reconcile(
            json!({"History":history.collect::<Vec<_>>(),"SpeechLanguage":"ru"})
                .as_object()
                .unwrap(),
            &Default::default(),
        )
        .unwrap();
        merge_builtin_history(dir.path(), &settings, &mut rows).unwrap();
        assert_eq!(rows.len(), 10_001);
        assert!(
            rows.iter()
                .skip(1)
                .all(|r| r.deleted && !r.approved && r.language == "ru")
        );
        let settings = kavychka_core::legacy::reconcile(
            json!({"History":["x".repeat(4001)]}).as_object().unwrap(),
            &Default::default(),
        )
        .unwrap();
        assert!(merge_builtin_history(dir.path(), &settings, &mut rows).is_err());
    }
    #[test]
    fn large_library_page_and_exact_lookup_keep_bounded_results_and_queue_order() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let tx = store.connection.transaction().unwrap();
        for i in 0..20000 {
            let mut row = phrase();
            row.id = format!("page-{i:05}");
            row.audio_path = None;
            row.model = "system".into();
            row.approved = true;
            insert(&tx, &row, false).unwrap();
        }
        tx.commit().unwrap();
        let (rows, total, page) = store.phrase_page(None, usize::MAX).unwrap();
        assert_eq!(total, 20000);
        assert_eq!(page, 399);
        assert_eq!(rows.len(), 50);
        assert!(serde_json::to_vec(&rows).unwrap().len() < 100000);
        assert_eq!(store.phrase("page-19999").unwrap().id, "page-19999");
        let row = phrase();
        let chosen = store
            .choose(
                row.day.as_deref().unwrap(),
                &row.category,
                row.followup,
                &row.language,
            )
            .unwrap()
            .unwrap();
        assert_eq!(chosen.id, "page-19999");
    }
    static IMPORT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/library-tests");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn fixture(root: &Path) -> PathBuf {
        let source = root.join("old");
        std::fs::create_dir_all(source.join("audio")).unwrap();
        std::fs::write(source.join("audio/ready.flac"), b"fLaC fixture").unwrap();
        std::fs::create_dir_all(source.join("presets/voice1")).unwrap();
        std::fs::write(
            source.join("presets/voice1/preset.json"),
            b"{\"version\":1,\"kind\":\"voice\",\"name\":\"Private\"}",
        )
        .unwrap();
        std::fs::write(source.join("settings.json"),serde_json::to_vec(&json!({"VoiceProvider":"Qwen local","ThemeMode":"dark","WorkStart":615,"PetSize":144,"ApiKey":"secret"})).unwrap()).unwrap();
        let c = rusqlite::Connection::open(source.join("library.sqlite3")).unwrap();
        c.execute_batch(include_str!("legacy_fixture.sql")).unwrap();
        source
    }
    #[test]
    fn bootstrap_preserves_history_assets_and_originals_and_is_once_only() {
        let _lock = IMPORT.lock().unwrap();
        let root = temp();
        let source = fixture(root.path());
        let before = preview::inventory(&source, &AtomicBool::new(false)).unwrap();
        let target = root.path().join("new");
        let mut store = Sandbox::open(&target).unwrap();
        let origin = store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(origin.imported_rows, 2);
        assert_eq!(store.load().unwrap().schedule.work_start, 615);
        assert!(store.load().unwrap().generation.text_enabled);
        assert_eq!(store.phrases().unwrap().len(), 2);
        assert_eq!(store.meta("voice_rotation").unwrap(), Some(json!(5)));
        assert!(store.phrase("excluded").unwrap().deleted);
        assert!(store.phrase("used").unwrap().used_at.is_some());
        assert_eq!(
            store.meta("legacy_settings").unwrap().unwrap()["desktop"]["PetSize"],
            144
        );
        assert!(
            !store
                .meta("legacy_settings")
                .unwrap()
                .unwrap()
                .to_string()
                .contains("secret")
        );
        store.select_phrase("used", false).unwrap();
        store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert!(store.phrase("used").unwrap().rejected);
        assert_eq!(store.phrases().unwrap().len(), 2);
        assert_eq!(
            preview::inventory(&source, &AtomicBool::new(false)).unwrap(),
            before
        );
        let offline = root.path().join("disconnected");
        assert!(
            source
                .canonicalize()
                .unwrap()
                .starts_with(root.path().canonicalize().unwrap())
        );
        assert!(
            std::path::absolute(&offline)
                .unwrap()
                .starts_with(std::path::absolute(root.path()).unwrap())
        );
        std::fs::rename(&source, &offline).unwrap();
        store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert!(!source.exists());
        assert_eq!(store.phrases().unwrap().len(), 2);
    }
    #[test]
    fn existing_settings_are_kept_while_missing_rows_are_imported() {
        let _lock = IMPORT.lock().unwrap();
        let root = temp();
        let source = fixture(root.path());
        let mut store = Sandbox::open(&root.path().join("new")).unwrap();
        let prefs = Preferences {
            user_name: "Current".into(),
            ..Default::default()
        };
        let mut prefs = prefs;
        prefs.desktop.pet_size = 222;
        prefs.desktop.run_at_login = true;
        prefs.desktop.automatic_updates = false;
        store.save(&prefs).unwrap();
        store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.load().unwrap().user_name, "Current");
        assert_eq!(store.phrases().unwrap().len(), 2);
        let state = State {
            pause_forever: true,
            ..Default::default()
        };
        store.save_runtime(&prefs, &state).unwrap();
        let utc = "2026-10-23T12:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let clock = kavychka_core::engine::Clock {
            utc,
            mono: 1000,
            date: utc.date_naive(),
        };
        store
            .complete_legacy_semantics(clock, |d, m| {
                Ok(d.and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)
                    .unwrap()
                    .and_utc())
            })
            .unwrap();
        assert!(store.load_state().unwrap().pause_forever);
        assert_eq!(store.load().unwrap().user_name, "Current");
        assert_eq!(store.load().unwrap().desktop.pet_size, 222);
        assert!(store.load().unwrap().desktop.run_at_login);
        assert!(!store.load().unwrap().desktop.automatic_updates);
        let mut current = store.load().unwrap();
        current.desktop.pet_size = 222;
        store.save(&current).unwrap();
        store
            .complete_legacy_semantics(clock, |_, _| unreachable!())
            .unwrap();
        assert_eq!(store.load().unwrap().desktop.pet_size, 222);
    }
    #[test]
    fn field_contract_distinguishes_archive_from_active_rows_and_id_conflicts() {
        let _lock = IMPORT.lock().unwrap();
        let root = temp();
        let source = fixture(root.path());
        let old = rusqlite::Connection::open(source.join("library.sqlite3")).unwrap();
        old.execute_batch("ALTER TABLE phrases ADD COLUMN historical_note TEXT; UPDATE phrases SET historical_note='archived extension'; PRAGMA user_version=2;").unwrap();
        drop(old);
        let before = preview::inventory(&source, &AtomicBool::new(false)).unwrap();
        let mut store = Sandbox::open(&root.path().join("new")).unwrap();
        let mut current = phrase();
        current.id = "used".into();
        current.text = "Keep the destination decision.".into();
        current.rejected = true;
        let tx = store.connection.transaction().unwrap();
        insert(&tx, &current, false).unwrap();
        tx.commit().unwrap();
        let origin = store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.phrase("used").unwrap().text, current.text);
        assert!(store.phrase("used").unwrap().rejected);
        let archive = rusqlite::Connection::open_with_flags(
            origin.snapshot.join("library.snapshot.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let archived: (String, String, String) = archive
            .query_row(
                "SELECT normalized,cache_key,historical_note FROM phrases WHERE id='used'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            archived,
            (
                "отдохни несколько минут".into(),
                "old-cache-key".into(),
                "archived extension".into()
            )
        );
        assert_eq!(
            archive
                .query_row("SELECT target FROM plans", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            13
        );
        assert_eq!(
            archive
                .query_row("SELECT attempts FROM preparation", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let active: String = store
            .connection
            .query_row("SELECT normalized FROM phrases WHERE id='used'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(active, current.text.to_lowercase());
        assert!(
            !serde_json::to_value(store.phrase("used").unwrap())
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("cache_key")
        );
        assert_eq!(
            preview::inventory(&source, &AtomicBool::new(false)).unwrap(),
            before
        );
    }
    #[test]
    fn base_phrase_schema_uses_historical_defaults_for_each_supported_sqlite_marker() {
        let _lock = IMPORT.lock().unwrap();
        for version in 0..=2 {
            let root = temp();
            let source = fixture(root.path());
            let db = rusqlite::Connection::open(source.join("library.sqlite3")).unwrap();
            for column in [
                "seed",
                "voice_style",
                "voice_instruction",
                "voice_reference",
                "followup",
                "language",
                "rejected",
            ] {
                db.execute_batch(&format!("ALTER TABLE phrases DROP COLUMN {column};"))
                    .unwrap();
            }
            db.pragma_update(None, "user_version", version).unwrap();
            drop(db);
            let mut store = Sandbox::open(&root.path().join("new")).unwrap();
            store
                .bootstrap_legacy(&source, &AtomicBool::new(false))
                .unwrap();
            let row = store.phrase("used").unwrap();
            assert_eq!(row.seed, 42);
            assert_eq!(row.language, "ru");
            assert!(!row.rejected && !row.followup);
            assert!(
                row.voice_style.is_empty()
                    && row.voice_instruction.is_empty()
                    && row.voice_reference.is_empty()
            );
        }
    }
    #[test]
    fn import_file_collision_preserves_destination_and_does_not_publish_origin() {
        let _lock = IMPORT.lock().unwrap();
        let root = temp();
        let source = fixture(root.path());
        let mut store = Sandbox::open(&root.path().join("new")).unwrap();
        let path = store.root.join("audio/ready.flac");
        std::fs::write(&path, b"destination audio").unwrap();
        assert!(
            store
                .bootstrap_legacy(&source, &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"destination audio");
        assert!(store.origin().unwrap().is_none());
        assert!(store.phrases().unwrap().is_empty());
    }
    fn phrase() -> Phrase {
        Phrase {
            id: "ready".into(),
            text: "Отложи работу и встань на минуту.".into(),
            category: "break".into(),
            model: "system".into(),
            profile: "system".into(),
            day: Some("2026-10-23".into()),
            audio_path: None,
            approved: true,
            deleted: false,
            rejected: false,
            used_at: None,
            created_at: "2026-10-23T00:00:00Z".into(),
            followup: false,
            language: "ru".into(),
            seed: 42,
            voice_style: String::new(),
            voice_instruction: String::new(),
            voice_reference: String::new(),
        }
    }
    #[test]
    fn preview_is_not_consumption_and_claim_is_atomic_with_reminder() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let p = phrase();
        store.accept_phrase(&p, None).unwrap();
        assert!(
            store
                .choose("2026-10-23", "break", false, "ru")
                .unwrap()
                .is_some()
        );
        assert!(store.phrase("ready").unwrap().used_at.is_none());
        let prefs = Preferences::default();
        let state = State::default();
        store.save_runtime(&prefs, &state).unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_state BEFORE UPDATE ON reminder_state BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
        assert!(
            store
                .save_delivery(&prefs, &state, Some(("notice1", "ready")))
                .is_err()
        );
        assert!(store.phrase("ready").unwrap().used_at.is_none());
        store
            .connection
            .execute_batch("DROP TRIGGER fail_state")
            .unwrap();
        store
            .save_delivery(&prefs, &state, Some(("notice1", "ready")))
            .unwrap();
        assert!(
            store
                .choose("2026-10-23", "break", false, "ru")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .save_delivery(&prefs, &state, Some(("notice2", "ready")))
                .is_err()
        );
    }
    #[test]
    fn daily_expiry_preserves_text_history_references_and_recovery_audio() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let staging = root.path().join("jobs/test");
        std::fs::create_dir_all(staging.join("audio")).unwrap();
        std::fs::write(staging.join("audio/test.flac"), b"fLaC test").unwrap();
        std::fs::write(
            root.path().join("audio/old-unreferenced.flac"),
            b"old cached voice",
        )
        .unwrap();
        std::fs::write(root.path().join("presets/reference.flac"), b"private ref").unwrap();
        let mut p = phrase();
        p.model = "tts-cosyvoice3".into();
        p.audio_path = Some("test.flac".into());
        store.accept_phrase(&p, Some(&staging)).unwrap();
        assert_eq!(store.expire_audio("2026-10-24", false).unwrap(), 1);
        let row = store.phrase("ready").unwrap();
        assert_eq!(row.text, p.text);
        assert!(row.audio_path.is_none());
        assert!(!root.path().join("audio/test.flac").exists());
        assert_eq!(
            std::fs::read(root.path().join("quarantine/2026-10-24/test.flac")).unwrap(),
            b"fLaC test"
        );
        assert!(root.path().join("presets/reference.flac").exists());
        assert!(!root.path().join("audio/old-unreferenced.flac").exists());
        assert_eq!(
            std::fs::read(
                root.path()
                    .join("quarantine/2026-10-24/old-unreferenced.flac")
            )
            .unwrap(),
            b"old cached voice"
        );
    }
    #[test]
    fn builtin_history_is_preserved_without_readding_external_duplicates() {
        let _lock = IMPORT.lock().unwrap();
        let root = temp();
        let source = fixture(root.path());
        std::fs::create_dir(source.join("builtin-days")).unwrap();
        let id = format!("stock-{}", "a".repeat(32));
        std::fs::write(source.join("builtin-days/2026-10-23.ru.json"),serde_json::to_vec(&json!({"Version":1,"Day":"2026-10-23","Language":"ru","Rows":[{"Id":id,"Text":"Расправь плечи и поднимись со стула.","Category":"exercise","CreatedAt":"2026-10-23T09:00:00Z","UsedAt":"2026-10-23T10:00:00Z","Followup":false},{"Id":format!("stock-{}","b".repeat(32)),"Kind":"external","Text":"Отдохни несколько минут.","Category":"break","Followup":false}]})).unwrap()).unwrap();
        let mut store = Sandbox::open(&root.path().join("new")).unwrap();
        store
            .bootstrap_legacy(&source, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.phrases().unwrap().len(), 3);
        assert!(store.phrase(&id).unwrap().deleted);
        assert!(
            store
                .choose("2026-10-23", "exercise", false, "ru")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn staging_cleanup_preserves_accepted_audio_and_logs_and_refuses_other_folders() {
        let root = temp();
        let store = Sandbox::open(root.path()).unwrap();
        let job = root.path().join("jobs/job-owned");
        std::fs::create_dir_all(job.join("audio")).unwrap();
        std::fs::write(job.join("audio/sample.flac"), b"staging").unwrap();
        std::fs::write(root.path().join("audio/sample.flac"), b"accepted").unwrap();
        std::fs::write(job.join("worker.log"), b"log").unwrap();
        std::fs::write(job.join("payload.json"), b"{}").unwrap();
        store.cleanup_staging(&job).unwrap();
        assert!(!job.join("audio").exists());
        assert!(!job.join("payload.json").exists());
        assert!(job.join("worker.log").exists());
        assert_eq!(
            std::fs::read(root.path().join("audio/sample.flac")).unwrap(),
            b"accepted"
        );
        assert!(store.cleanup_staging(&root.path().join("presets")).is_err());
    }
}
