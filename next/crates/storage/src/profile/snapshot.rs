use super::*;
use crate::{
    library::{self, Phrase},
    sandbox::{self, Sandbox},
};
use kavychka_core::{engine::State, preferences::Preferences};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::collections::BTreeMap;
pub(crate) struct Snapshot {
    pub initialized: bool,
    pub prefs: Preferences,
    pub state: State,
    pub phrases: Vec<Phrase>,
    pub meta: BTreeMap<String, Value>,
    pub claims: BTreeMap<String, String>,
}
fn schema(db: &Connection) -> Result<BTreeMap<String, String>> {
    let mut q =
        db.prepare("SELECT type||':'||name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|r| {
        r.map(|(k, v)| {
            (
                k,
                v.chars()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<String>()
                    .to_ascii_lowercase(),
            )
        })
        .map_err(Error::from)
    })
    .collect()
}
pub(crate) fn read(path: &Path) -> Result<Snapshot> {
    crate::preview::no_link_chain(path)?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(std::time::Duration::from_secs(2))?;
    db.execute_batch("BEGIN")?;
    db.pragma_update(None, "trusted_schema", false)?;
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(sandbox::BASE_SQL)?;
    reference.execute_batch(library::LIBRARY_SQL)?;
    if db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))? != 3
        || schema(&db)? != schema(&reference)?
    {
        return Err(invalid(
            "unsupported profile SQLite schema; no migration was applied",
        ));
    }
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(invalid("profile SQLite integrity failed"));
    }
    for table in ["phrases", "library_meta", "delivery_claims"] {
        let count: i64 =
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        if count > 100000 {
            return Err(invalid("profile exceeds 100,000 rows in a table"));
        }
    }
    for table in ["preferences", "reminder_state"] {
        let count: i64 =
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        if count > 1 {
            return Err(invalid("profile singleton table has extra rows"));
        }
    }
    for (table, limit) in [
        ("preferences", 1_048_576i64),
        ("reminder_state", 134_217_728),
        ("library_meta", 8_388_608),
    ] {
        let maximum: i64 = db.query_row(
            &format!("SELECT COALESCE(MAX(length(json)),0) FROM {table}"),
            [],
            |r| r.get(0),
        )?;
        if maximum > limit {
            return Err(invalid("profile JSON cell exceeds its memory limit"));
        }
    }
    let pref: Option<String> = db
        .query_row("SELECT json FROM preferences WHERE id=1", [], |r| r.get(0))
        .optional()?;
    let initialized = pref.is_some();
    let mut prefs: Preferences = pref
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or_default();
    prefs.validate().map_err(invalid)?;
    let state: Option<String> = db
        .query_row("SELECT json FROM reminder_state WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    let state: State = state
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or_default();
    state.validate().map_err(invalid)?;
    let phrases = library::read_phrases(&db)?;
    for p in &phrases {
        library::validate_phrase(p)?;
    }
    let mut meta = BTreeMap::new();
    let mut q = db.prepare("SELECT key,json FROM library_meta")?;
    for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (key, json) = row?;
        let mut value: Value = serde_json::from_str(&json)?;
        if key == "legacy_settings" {
            let old: kavychka_core::legacy::SettingsSnapshot = serde_json::from_value(value)?;
            if old.format != 1 {
                return Err(invalid("unsupported legacy settings metadata"));
            }
            let desktop: serde_json::Map<String, Value> = old.desktop.into_iter().collect();
            value =
                serde_json::to_value(kavychka_core::legacy::reconcile(&desktop, &old.inference)?)?;
        } else if key == "origin" || key == "profile_origin_hint" {
            let origin: crate::library::Origin = serde_json::from_value(value)?;
            if !origin.source.is_absolute() || !origin.snapshot.is_absolute() {
                return Err(invalid("invalid archived origin hint"));
            }
            value = serde_json::to_value(origin)?;
        } else if key == "recovery_path" || key == "profile_recovery_hint" {
            if !value.as_str().is_some_and(|s| Path::new(s).is_absolute()) {
                return Err(invalid("invalid recovery path hint"));
            }
        } else if key.starts_with("profile_restore:") || key.starts_with("profile_history:") {
            let suffix = key.split_once(':').unwrap().1;
            if suffix.len() != 64
                || !suffix.bytes().all(|b| b.is_ascii_hexdigit())
                || value["format"] != 1
            {
                return Err(invalid("unsupported profile history marker"));
            }
            let fields = if key.starts_with("profile_restore:") {
                &["format", "created_utc", "backup", "phrases"][..]
            } else {
                &["format", "snapshot", "audio", "presets"][..]
            };
            value
                .as_object_mut()
                .ok_or_else(|| invalid("invalid profile history marker"))?
                .retain(|k, _| fields.contains(&k.as_str()));
            if key.starts_with("profile_history:")
                && value["snapshot"].as_str()
                    != Some(format!("legacy/profiles/{suffix}/library.sqlite3").as_str())
            {
                return Err(invalid("invalid historical profile reference"));
            }
        } else if let Some(id) = key.strip_prefix("purpose:") {
            library::safe_name(id)?;
            if !matches!(value.as_str(), Some("daily" | "batch" | "runtime")) {
                return Err(invalid("unknown preparation purpose"));
            }
        } else if let Some(id) = key.strip_prefix("pending_notice:") {
            let notice: kavychka_core::engine::Notice = serde_json::from_value(value.clone())?;
            if id != notice.key
                || id.is_empty()
                || id.len() > 4096
                || id.chars().any(char::is_control)
            {
                return Err(invalid("invalid archived pending notice"));
            }
            State {
                pending: vec![notice],
                ..Default::default()
            }
            .validate()
            .map_err(invalid)?;
        } else if key == "support_day" || key == "support_history" {
            crate::support::validate_value(&key, &value).map_err(invalid)?;
        } else if key.starts_with("delivery_journal:") {
            let journal: crate::delivery::DeliveryJournal = serde_json::from_value(value.clone())?;
            if journal.format != 1 || key != format!("delivery_journal:{}", journal.notice) {
                return Err(invalid("invalid delivery journal"));
            }
        } else if !(key.starts_with("purpose:")
            || key.starts_with("profile_restore:")
            || key.starts_with("profile_history:")
            || [
                "origin",
                "profile_origin_hint",
                "profile_recovery_hint",
                "legacy_semantics_v2",
                "legacy_keep_current_preferences",
                "legacy_preparation_states",
                "import_limit",
                "pending_recovery_limit",
                "voice_rotation",
                "preparation_state",
                "audio_day",
                "recovery_path",
            ]
            .contains(&key.as_str()))
        {
            return Err(invalid(format!("unsupported profile metadata key: {key}")));
        }
        meta.insert(key, value);
    }
    if !initialized && let Some(limit) = meta.get("pending_recovery_limit") {
        prefs.desktop.recovery_limit_bytes = limit
            .as_u64()
            .ok_or_else(|| invalid("invalid pending recovery limit"))?;
        prefs.validate().map_err(invalid)?;
    }
    let ids: std::collections::BTreeSet<_> = phrases.iter().map(|p| p.id.as_str()).collect();
    let mut claims = BTreeMap::new();
    let mut q = db.prepare("SELECT notice,phrase_id FROM delivery_claims")?;
    for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (notice, id) = row?;
        if notice.is_empty()
            || notice.len() > 4096
            || notice.chars().any(char::is_control)
            || !ids.contains(id.as_str())
        {
            return Err(invalid("invalid delivery claim"));
        }
        claims.insert(notice, id);
    }
    Ok(Snapshot {
        initialized,
        prefs,
        state,
        phrases,
        meta,
        claims,
    })
}
pub(crate) fn write(path: &Path, snapshot: &Snapshot) -> Result<()> {
    if path.exists() {
        return Err(invalid("snapshot target must be new"));
    }
    let mut db = Connection::open(path)?;
    db.execute_batch(sandbox::BASE_SQL)?;
    db.execute_batch(library::LIBRARY_SQL)?;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO preferences VALUES(1,?1)",
        [serde_json::to_string(&snapshot.prefs)?],
    )?;
    tx.execute(
        "INSERT INTO reminder_state VALUES(1,?1)",
        [serde_json::to_string(&snapshot.state)?],
    )?;
    for row in &snapshot.phrases {
        library::insert(&tx, row, true)?;
    }
    for (key, value) in &snapshot.meta {
        tx.execute(
            "INSERT INTO library_meta VALUES(?1,?2)",
            params![key, serde_json::to_string(value)?],
        )?;
    }
    for (notice, id) in &snapshot.claims {
        tx.execute(
            "INSERT INTO delivery_claims VALUES(?1,?2)",
            params![notice, id],
        )?;
    }
    tx.commit()?;
    db.close().map_err(|(_, e)| e)?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}
pub(crate) fn assets(root: &Path, snapshot: &Snapshot) -> Result<()> {
    for p in &snapshot.phrases {
        if let Some(name) = &p.audio_path {
            let file = crate::presets::inside(&root.join("audio"), name)?;
            if !file.is_file() {
                return Err(invalid("profile has a missing audio reference"));
            }
        }
    }
    let (presets, warnings) = crate::presets::scan(root)?;
    if !warnings.is_empty() {
        return Err(invalid("profile has an invalid preset"));
    }
    for id in [
        &snapshot.prefs.desktop.character_preset,
        &snapshot.prefs.desktop.visual_preset,
        &snapshot.prefs.generation.voice_profile,
    ] {
        if !id.is_empty() && !presets.iter().any(|p| &p.id == id) {
            return Err(invalid("profile has a missing selected preset"));
        }
    }
    Ok(())
}
pub(crate) fn current(store: &Sandbox) -> Result<Snapshot> {
    let mut meta = BTreeMap::new();
    let mut q = store
        .connection
        .prepare("SELECT key,json FROM library_meta")?;
    for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, v) = row?;
        meta.insert(k, serde_json::from_str(&v)?);
    }
    let mut claims = BTreeMap::new();
    let mut q = store
        .connection
        .prepare("SELECT notice,phrase_id FROM delivery_claims")?;
    for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, v) = row?;
        claims.insert(k, v);
    }
    let initialized =
        store
            .connection
            .query_row("SELECT EXISTS(SELECT 1 FROM preferences)", [], |r| r.get(0))?;
    Ok(Snapshot {
        initialized,
        prefs: store.load()?,
        state: store.load_state()?,
        phrases: store.phrases()?,
        meta,
        claims,
    })
}
