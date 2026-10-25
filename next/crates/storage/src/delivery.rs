//! Idempotent delivery facts in the existing, extensible library_meta table.
//! Journal facts omit phrase text, media paths and key codes. Separately archived
//! pending notices retain their original text for explicit user review.
use crate::{Error, Result, sandbox::Sandbox};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryJournal {
    pub format: u32,
    pub notice: String,
    pub event_ids: Vec<String>,
    pub phrase_id: Option<String>,
    pub phases: BTreeMap<String, String>,
}
fn key(notice: &str) -> Result<String> {
    if notice.is_empty() || notice.len() > 4096 || notice.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid notice ID".into()));
    }
    Ok(format!("delivery_journal:{notice}"))
}
pub(crate) fn archive_notice(
    connection: &rusqlite::Connection,
    notice: &kavychka_core::engine::Notice,
) -> Result<()> {
    key(&notice.key)?;
    let state = kavychka_core::engine::State {
        pending: vec![notice.clone()],
        ..Default::default()
    };
    state.validate().map_err(Error::Invalid)?;
    connection.execute(
        "INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
        rusqlite::params![
            format!("pending_notice:{}", notice.key),
            serde_json::to_string(notice)?
        ],
    )?;
    Ok(())
}
pub(crate) fn write_phase(
    connection: &rusqlite::Connection,
    notice: &str,
    phase: &str,
    events: &[String],
    phrase: Option<&str>,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    if ![
        "committed",
        "shown",
        "audio_requested",
        "audio_preparing",
        "audio_started",
        "audio_completed",
        "audio_cancelled",
        "audio_failed",
        "confirmed",
    ]
    .contains(&phase)
    {
        return Err(Error::Invalid("unknown delivery phase".into()));
    }
    let key = key(notice)?;
    let old: Option<String> = connection
        .query_row("SELECT json FROM library_meta WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .optional()?;
    let mut journal = match old {
        Some(json) => serde_json::from_str::<DeliveryJournal>(&json)?,
        None => DeliveryJournal {
            format: 1,
            notice: notice.into(),
            event_ids: events.to_vec(),
            phrase_id: phrase.map(str::to_owned),
            phases: BTreeMap::new(),
        },
    };
    if journal.format != 1 || journal.notice != notice {
        return Err(Error::Invalid(
            "newer or mismatched delivery journal; not resetting it".into(),
        ));
    }
    let mut enriched = false;
    if journal.event_ids.is_empty() && !events.is_empty() {
        journal.event_ids = events.to_vec();
        enriched = true;
    }
    if let Some(phrase) = phrase {
        if journal.phrase_id.as_deref().is_some_and(|id| id != phrase) {
            return Err(Error::Invalid("delivery phrase identity changed".into()));
        }
        enriched |= journal.phrase_id.is_none();
        journal.phrase_id = Some(phrase.into());
    }
    if journal.phases.contains_key(phase) && !enriched {
        return Ok(());
    }
    journal
        .phases
        .entry(phase.into())
        .or_insert_with(|| chrono::Utc::now().to_rfc3339());
    connection.execute(
        "INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET json=excluded.json",
        rusqlite::params![key, serde_json::to_string(&journal)?],
    )?;
    Ok(())
}
impl Sandbox {
    pub fn pending_archive_page(
        &self,
        page: usize,
    ) -> Result<(Vec<kavychka_core::engine::Notice>, usize, usize)> {
        let total = self.connection.query_row(
            "SELECT count(*) FROM library_meta WHERE key GLOB 'pending_notice:*'",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let page = page.min(total.saturating_sub(1) / 10);
        let mut q=self.connection.prepare("SELECT json FROM library_meta WHERE key GLOB 'pending_notice:*' ORDER BY json_extract(json,'$.created') DESC,key LIMIT 10 OFFSET ?1")?;
        let rows = q
            .query_map([page as i64 * 10], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((
            rows.iter()
                .map(|s| serde_json::from_str(s))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            total,
            page,
        ))
    }
    pub fn confirm_archived(&mut self, notice: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        if tx.execute(
            "DELETE FROM library_meta WHERE key=?1",
            [format!("pending_notice:{notice}")],
        )? > 0
        {
            write_phase(&tx, notice, "confirmed", &[], None)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn delivery_phase(&mut self, notice: &str, phase: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        write_phase(&tx, notice, phase, &[], None)?;
        tx.commit()?;
        Ok(())
    }
    pub fn delivery_journal(&self, notice: &str) -> Result<Option<DeliveryJournal>> {
        self.meta(&key(notice)?)?
            .map(serde_json::from_value)
            .transpose()
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kavychka_core::{
        engine::{Clock, Engine},
        preferences::Preferences,
    };
    #[test]
    fn archived_notices_commit_with_state_survive_profile_export_and_confirm_once() {
        let (dir, mut store, prefs, mut engine, notice) = fixture();
        let clock = Clock {
            utc: notice.created,
            mono: 1000,
            date: notice.created.date_naive(),
        };
        for _ in 0..132 {
            engine.manual_stretch(clock);
        }
        assert!(!engine.archived_pending.is_empty());
        store.connection.execute_batch("CREATE TRIGGER reject_state BEFORE INSERT ON reminder_state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(
            store
                .save_delivery_archived(
                    &prefs,
                    &engine.state,
                    None,
                    None,
                    None,
                    &engine.archived_pending
                )
                .is_err()
        );
        assert_eq!(store.pending_archive_page(0).unwrap().1, 0);
        store
            .connection
            .execute_batch("DROP TRIGGER reject_state")
            .unwrap();
        store
            .save_delivery_archived(
                &prefs,
                &engine.state,
                None,
                None,
                None,
                &engine.archived_pending,
            )
            .unwrap();
        let (page, total, _) = store.pending_archive_page(0).unwrap();
        assert_eq!(total, engine.archived_pending.len());
        assert!(!page.is_empty());
        let expected = page[0].clone();
        let output = dir.path().join("archive.zip");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        crate::profile::CaptureSource::new(&store)
            .unwrap()
            .export(
                &output,
                &dir.path().join("export-job"),
                Default::default(),
                "0.1.24",
                &cancel,
                |_| {},
            )
            .unwrap();
        let inspected = crate::profile::inspect(
            &output,
            &dir.path().join("inspect-job"),
            2_000_000_000,
            &cancel,
            |_| {},
        )
        .unwrap();
        let mut target = Sandbox::open(&dir.path().join("restored")).unwrap();
        let mut plan = crate::profile::RestorePlan::prepare(&target, inspected, &cancel).unwrap();
        plan.prepare_files(
            crate::profile::CaptureSource::new(&target).unwrap(),
            "0.1.24",
            &cancel,
            |_| {},
        )
        .unwrap();
        plan.apply(&mut target, &cancel).unwrap();
        assert_eq!(target.pending_archive_page(0).unwrap().1, total);
        assert_eq!(
            target.pending_archive_page(0).unwrap().0[0].text,
            expected.text
        );
        store.confirm_archived(&expected.key).unwrap();
        store.confirm_archived(&expected.key).unwrap();
        assert_eq!(store.pending_archive_page(0).unwrap().1, total - 1);
        assert!(
            store
                .delivery_journal(&expected.key)
                .unwrap()
                .unwrap()
                .phases
                .contains_key("confirmed")
        );
        assert_eq!(store.load_state().unwrap().pending.len(), 128);
    }
    fn fixture() -> (
        tempfile::TempDir,
        Sandbox,
        Preferences,
        Engine,
        kavychka_core::engine::Notice,
    ) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/p4-storage-tests");
        std::fs::create_dir_all(&path).unwrap();
        let dir = tempfile::tempdir_in(path).unwrap();
        let mut store = Sandbox::open(dir.path()).unwrap();
        let prefs = Preferences::default();
        let utc = "2026-10-23T09:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let mut engine = Engine::new(
            prefs.clone(),
            Default::default(),
            Clock {
                utc,
                mono: 1000,
                date: utc.date_naive(),
            },
        )
        .unwrap();
        let notice = engine
            .manual_stretch(Clock {
                utc,
                mono: 1000,
                date: utc.date_naive(),
            })
            .announce
            .unwrap();
        store
            .accept_phrase(
                &crate::library::Phrase {
                    id: "ready1".into(),
                    text: "PRIVATE PHRASE MUST NOT ENTER DELIVERY JOURNAL".into(),
                    category: "exercise".into(),
                    model: "system".into(),
                    profile: "system".into(),
                    day: Some(utc.date_naive().to_string()),
                    audio_path: None,
                    approved: true,
                    deleted: false,
                    rejected: false,
                    used_at: None,
                    created_at: utc.to_rfc3339(),
                    followup: false,
                    language: "ru".into(),
                    seed: 42,
                    voice_style: String::new(),
                    voice_instruction: String::new(),
                    voice_reference: String::new(),
                },
                None,
            )
            .unwrap();
        (dir, store, prefs, engine, notice)
    }
    #[test]
    fn claim_state_and_journal_are_atomic_and_facts_are_idempotent_without_phrase_text() {
        let (_dir, mut store, prefs, engine, notice) = fixture();
        store
            .save_delivery_checkpoint(
                &prefs,
                &engine.state,
                Some((&notice.key, "ready1")),
                Some(&notice),
                None,
            )
            .unwrap();
        let journal = store.delivery_journal(&notice.key).unwrap().unwrap();
        assert_eq!(journal.event_ids, [notice.events[0].id.clone()]);
        assert_eq!(journal.phrase_id.as_deref(), Some("ready1"));
        assert_eq!(journal.phases.len(), 1);
        store.delivery_phase(&notice.key, "shown").unwrap();
        let shown = store.delivery_journal(&notice.key).unwrap().unwrap();
        store.delivery_phase(&notice.key, "shown").unwrap();
        assert_eq!(
            shown.phases,
            store.delivery_journal(&notice.key).unwrap().unwrap().phases
        );
        assert!(
            !serde_json::to_string(&shown)
                .unwrap()
                .contains("PRIVATE PHRASE")
        );
        assert!(
            store
                .delivery_phase(&notice.key, "future-unknown-phase")
                .is_err()
        );
    }
    #[test]
    fn failed_state_write_rolls_back_claim_and_journal_together() {
        let (_dir, mut store, prefs, engine, notice) = fixture();
        store.connection.execute_batch("CREATE TRIGGER reject_state BEFORE INSERT ON reminder_state BEGIN SELECT RAISE(ABORT,'P4 fixture'); END;").unwrap();
        assert!(
            store
                .save_delivery_checkpoint(
                    &prefs,
                    &engine.state,
                    Some((&notice.key, "ready1")),
                    Some(&notice),
                    None
                )
                .is_err()
        );
        assert!(store.phrase("ready1").unwrap().used_at.is_none());
        assert!(store.delivery_journal(&notice.key).unwrap().is_none());
        assert!(store.load_state().unwrap().pending.is_empty());
    }
    #[test]
    fn confirmed_phase_is_in_the_same_transaction_as_removing_the_pending_message() {
        let (_dir, mut store, prefs, mut engine, notice) = fixture();
        store
            .save_delivery_checkpoint(
                &prefs,
                &engine.state,
                Some((&notice.key, "ready1")),
                Some(&notice),
                None,
            )
            .unwrap();
        engine.state.pending.clear();
        store.connection.execute_batch("CREATE TRIGGER reject_state BEFORE INSERT ON reminder_state BEGIN SELECT RAISE(ABORT,'P4 fixture'); END;").unwrap();
        assert!(
            store
                .save_delivery_checkpoint(&prefs, &engine.state, None, None, Some(&notice.key))
                .is_err()
        );
        assert!(
            !store
                .delivery_journal(&notice.key)
                .unwrap()
                .unwrap()
                .phases
                .contains_key("confirmed")
        );
        assert_eq!(store.load_state().unwrap().pending.len(), 1);
        store
            .connection
            .execute_batch("DROP TRIGGER reject_state;")
            .unwrap();
        store
            .save_delivery_checkpoint(&prefs, &engine.state, None, None, Some(&notice.key))
            .unwrap();
        assert!(
            store
                .delivery_journal(&notice.key)
                .unwrap()
                .unwrap()
                .phases
                .contains_key("confirmed")
        );
        assert!(store.load_state().unwrap().pending.is_empty());
    }
}
