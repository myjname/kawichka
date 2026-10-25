//! Isolated P3 development store. Cannot open a live or preview-import directory.
use crate::{Error, Result, preview::no_link_chain};
use kavychka_core::preferences::Preferences;
use rusqlite::{Connection, OptionalExtension};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

pub(crate) const BASE_SQL: &str = "BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS preferences(id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS reminder_state(id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL); PRAGMA user_version=2; COMMIT;";
const OWNER: &str = "kavychka-shell-sandbox-v1\n";
pub struct Sandbox {
    pub(crate) connection: Connection,
    pub root: std::path::PathBuf,
    _lock: File,
}
impl Sandbox {
    pub fn backup_database(&self, to: &Path) -> Result<()> {
        no_link_chain(to)?;
        if to.exists() {
            return Err(Error::Invalid("backup database must be new".into()));
        }
        let mut out = Connection::open(to)?;
        {
            let backup = rusqlite::backup::Backup::new(&self.connection, &mut out)?;
            backup.run_to_completion(128, std::time::Duration::from_millis(5), None)?;
        }
        let integrity: String = out.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::Invalid("backup integrity failed".into()));
        }
        out.close().map_err(|(_, e)| e)?;
        Ok(())
    }
    pub fn open(path: &Path) -> Result<Self> {
        let path = crate::transfer::resolve(path)?;
        Self::open_unresolved(&path)
    }
    pub(crate) fn open_unresolved(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::Invalid("absolute sandbox path required".into()));
        }
        no_link_chain(path)?;
        if !path.try_exists()? {
            std::fs::create_dir_all(path)?;
        }
        let owner = path.join("shell-sandbox.owner");
        // Refuse arbitrary nonempty folders, including the live store, before creating anything.
        if owner.try_exists()? {
            no_link_chain(&owner)?;
            if std::fs::read_to_string(&owner)? != OWNER {
                return Err(Error::Invalid("unknown sandbox owner".into()));
            }
        } else {
            if std::fs::read_dir(path)?.next().is_some() {
                return Err(Error::Invalid(
                    "sandbox must be empty; live data is not accepted".into(),
                ));
            }
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&owner)?;
            file.write_all(OWNER.as_bytes())?;
            file.sync_all()?;
        }
        let lock_path = path.join("shell.lock");
        no_link_chain(&lock_path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock().map_err(|_| Error::Busy)?;
        let ignore = path.join(".gitignore");
        no_link_chain(&ignore)?;
        if !ignore.try_exists()? {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(ignore)?;
            file.write_all(b"*\n")?;
            file.sync_all()?;
        }
        let db = path.join("shell.sqlite3");
        for suffix in ["", "-wal", "-shm", "-journal"] {
            no_link_chain(&path.join(format!("shell.sqlite3{suffix}")))?;
        }
        let connection = Connection::open(db)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 3 {
            return Err(Error::Invalid("newer sandbox schema; no downgrade".into()));
        }
        if version < 2 {
            connection.execute_batch(BASE_SQL)?;
        }
        let mut store = Self {
            connection,
            root: path.to_path_buf(),
            _lock: lock,
        };
        store.initialize_library()?;
        crate::profile::recover(&mut store)?;
        Ok(store)
    }
    pub fn load(&self) -> Result<Preferences> {
        let json: Option<String> = self
            .connection
            .query_row("SELECT json FROM preferences WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let fresh = json.is_none();
        let mut prefs = match json {
            Some(json) => serde_json::from_str(&json)?,
            None => Preferences::default(),
        };
        if fresh && let Some(limit) = self.meta("pending_recovery_limit")? {
            prefs.desktop.recovery_limit_bytes = limit
                .as_u64()
                .ok_or_else(|| Error::Invalid("invalid pending recovery limit".into()))?;
        }
        prefs.validate().map_err(Error::Invalid)?;
        Ok(prefs)
    }
    pub fn save(&mut self, prefs: &Preferences) -> Result<()> {
        prefs.validate().map_err(Error::Invalid)?;
        let json = serde_json::to_string(prefs)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [json],
        )?;
        tx.execute(
            "DELETE FROM library_meta WHERE key='pending_recovery_limit'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn has_preferences(&self) -> Result<bool> {
        Ok(self
            .connection
            .query_row("SELECT EXISTS(SELECT 1 FROM preferences)", [], |r| r.get(0))?)
    }
    pub fn load_state(&self) -> Result<kavychka_core::engine::State> {
        let json: Option<String> = self
            .connection
            .query_row("SELECT json FROM reminder_state WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let state = match json {
            Some(json) => serde_json::from_str(&json)?,
            None => kavychka_core::engine::State::default(),
        };
        state.validate().map_err(Error::Invalid)?;
        Ok(state)
    }
    pub fn save_runtime(
        &mut self,
        prefs: &Preferences,
        state: &kavychka_core::engine::State,
    ) -> Result<()> {
        prefs.validate().map_err(Error::Invalid)?;
        state.validate().map_err(Error::Invalid)?;
        let prefs = serde_json::to_string(prefs)?;
        let state = serde_json::to_string(state)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO preferences VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            [prefs],
        )?;
        tx.execute("INSERT INTO reminder_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",[state])?;
        tx.execute(
            "DELETE FROM library_meta WHERE key='pending_recovery_limit'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kavychka_core::preferences::{Language, Theme};
    #[test]
    fn persists_unicode_and_schedule_rejects_invalid_without_losing_saved_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Sandbox::open(temp.path()).unwrap();
        let mut prefs = store.load().unwrap();
        prefs.theme = Theme::Dark;
        prefs.ui_language = Language::En;
        prefs.user_name = "Алёна".into();
        prefs.schedule.work_days = vec![0, 6];
        prefs.schedule.break_every = 90;
        store.save(&prefs).unwrap();
        prefs.schedule.break_every = 0;
        assert!(store.save(&prefs).is_err());
        drop(store);
        let loaded = Sandbox::open(temp.path()).unwrap().load().unwrap();
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.user_name, "Алёна");
        assert_eq!(loaded.ui_language, Language::En);
        assert_eq!(loaded.schedule.work_days, [0, 6]);
        assert_eq!(loaded.schedule.break_every, 90);
    }
    #[test]
    fn refuses_existing_data_second_owner_and_corruption() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("settings.json"), "private").unwrap();
        assert!(Sandbox::open(temp.path()).is_err());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        let isolated = tempfile::tempdir().unwrap();
        let store = Sandbox::open(isolated.path()).unwrap();
        assert!(matches!(Sandbox::open(isolated.path()), Err(Error::Busy)));
        store
            .connection
            .execute("INSERT INTO preferences VALUES(1,'broken')", [])
            .unwrap();
        assert!(store.load().is_err());
        drop(store);
        assert!(Sandbox::open(isolated.path()).unwrap().load().is_err());
    }
    #[test]
    fn future_preferences_are_rejected_without_resetting_their_bytes() {
        let root = tempfile::tempdir().unwrap();
        let store = Sandbox::open(root.path()).unwrap();
        let mut prefs = serde_json::to_value(Preferences::default()).unwrap();
        prefs["format"] = serde_json::json!(2);
        prefs["future_personal_setting"] = serde_json::json!("keep me");
        let json = serde_json::to_string(&prefs).unwrap();
        store
            .connection
            .execute("INSERT INTO preferences VALUES(1,?1)", [&json])
            .unwrap();
        drop(store);
        let store = Sandbox::open(root.path()).unwrap();
        assert!(store.load().is_err());
        let actual: String = store
            .connection
            .query_row("SELECT json FROM preferences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(actual, json);
    }
    #[test]
    fn settings_and_reminder_state_commit_atomically_even_if_second_write_fails() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Sandbox::open(root.path()).unwrap();
        let original = Preferences::default();
        let state = kavychka_core::engine::State::default();
        store.save_runtime(&original, &state).unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_state BEFORE UPDATE ON reminder_state BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        let mut changed = original.clone();
        changed.user_name = "Не должно сохраниться".into();
        assert!(store.save_runtime(&changed, &state).is_err());
        assert_eq!(store.load().unwrap().user_name, original.user_name);
        assert_eq!(
            serde_json::to_string(&store.load_state().unwrap()).unwrap(),
            serde_json::to_string(&state).unwrap()
        );
    }
}
