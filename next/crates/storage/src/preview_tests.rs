use super::*;
use std::process::Command;

fn temp() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/rust-next-tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
fn cancel() -> AtomicBool {
    AtomicBool::new(false)
}
#[test]
fn legacy_version_and_schema_boundaries_are_explicit() {
    for version in [-1, 0, 1, 2, 3] {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        assert_eq!(validate_database(&db).is_ok(), (0..=2).contains(&version));
    }
    for sql in [
        "CREATE TABLE unknown_data(id TEXT)",
        "CREATE VIEW unknown_view AS SELECT 1",
        "CREATE TABLE settings(key TEXT,value TEXT); CREATE TRIGGER changed AFTER INSERT ON settings BEGIN DELETE FROM settings; END",
    ] {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(sql).unwrap();
        assert!(validate_database(&db).is_err());
    }
}
fn run(source: &Path, target: &Path) -> Result<Report> {
    prepare_steps(source, target, 2_000_000_000, &cancel(), |_| {})
}
fn fixture(root: &Path) -> PathBuf {
    let source = root.join("Исходник с пробелами");
    fs::create_dir(&source).unwrap();
    fs::write(source.join(".test-fixture"), b"owned test fixture").unwrap();
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend(
        serde_json::to_vec(&json!({
            "SchemaVersion":2,"BackendSettingsMigrated":false,"BackendDailySettingsMigrated":true,
            "BackendSettingsOverrides":["text_model"],"TextModel":"text-qwen35-4b",
            "AutomaticPreparation":false,"ThemeMode":"dark","UiLanguage":"ru","SpeechLanguage":"en",
            "TimeZoneId":"Central Europe Standard Time","WorkDays":[1,3,5],
            "ExerciseUtc":"/Date(1792756800000)/","ApiKey":"fixture-secret"
        }))
        .unwrap(),
    );
    fs::write(source.join("settings.json"), bytes).unwrap();
    let db = Connection::open(source.join("library.sqlite3")).unwrap();
    db.execute_batch(include_str!("legacy_fixture.sql"))
        .unwrap();
    for (key, value) in [
        ("text_model", json!("text-gemma4-12b")),
        ("tts_model", json!("tts-cosyvoice3")),
        ("swear", json!(true)),
        ("auto_daily", json!(true)),
        (
            "model_paths",
            json!({"tts-cosyvoice3":source.join("models/tts-cosyvoice3")}),
        ),
        (
            "character_description",
            json!("Нейтральный тестовый персонаж."),
        ),
        (
            "delivery_catalog",
            json!([{"id":"custom1","name":"Своя подача","instruction":"Чётко"}]),
        ),
        ("api_token", json!("fixture-secret")),
    ] {
        db.execute(
            "INSERT INTO settings VALUES(?1,?2)",
            [key, &value.to_string()],
        )
        .unwrap();
    }
    drop(db);
    for dir in [
        "audio",
        "presets/voice1",
        "builtin-days",
        "models/tts-cosyvoice3",
    ] {
        fs::create_dir_all(source.join(dir)).unwrap();
    }
    for (file, bytes) in [
        (
            "audio/ready.flac",
            b"fLaC fixture: byte preservation, not playback".as_slice(),
        ),
        ("audio/orphan.flac", b"unlinked recording must survive"),
        ("presets/voice1/one.wav", b"RIFF fixture reference bytes"),
        (
            "builtin-days/2026-10-22.ru.json",
            b"{\"Version\":1,\"Rows\":[]}",
        ),
        ("reminder-state.json", b"{\"Version\":1,\"Pending\":true}"),
        (
            "models/tts-cosyvoice3/model.bin",
            b"do not duplicate this model",
        ),
    ] {
        fs::write(source.join(file), bytes).unwrap();
    }
    fs::write(source.join("presets/voice1/preset.json"),serde_json::to_vec(&json!({
        "version":1,"kind":"voice","name":"Test voice","references":[{"file":"one.wav","transcript":"Test."}]
    })).unwrap()).unwrap();
    source
}

#[test]
fn preserves_rows_audio_presets_history_and_model_paths_without_touching_source() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("Preview");
    let before = inventory(&source, &cancel()).unwrap();
    let report = run(&source, &target).unwrap();
    assert_eq!(report.phase, Phase::Ready);
    assert!(!report.models_copied);
    assert!(!report.live_storage_switched);
    assert_eq!(report.rows["phrases"], 2);
    assert!(!target.join("models").exists());
    assert_eq!(
        report.model_roots["tts-cosyvoice3"],
        source.join("models/tts-cosyvoice3").to_string_lossy()
    );
    assert_eq!(inventory(&source, &cancel()).unwrap(), before);
    let copied = Connection::open(target.join("library.snapshot.sqlite3")).unwrap();
    let settings: SettingsSnapshot = serde_json::from_str(
        &copied
            .query_row("SELECT json FROM app_settings", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings.desktop["TextModel"], "text-qwen35-4b");
    assert_eq!(settings.desktop["AutomaticPreparation"], false);
    assert_eq!(settings.desktop["SpeechLanguage"], "en");
    assert_eq!(settings.desktop["Swear"], true);
    assert_eq!(
        settings.inference["model_paths"]["tts-cosyvoice3"],
        json!(source.join("models/tts-cosyvoice3"))
    );
    for (query, expected) in [
        ("SELECT next_index FROM voice_rotation", 5),
        (
            "SELECT deleted+rejected FROM phrases WHERE id='excluded'",
            2,
        ),
        ("SELECT attempts FROM preparation", 2),
        ("SELECT COUNT(*) FROM settings", 0),
    ] {
        assert_eq!(
            copied.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap(),
            expected
        );
    }
    assert_eq!(
        copied
            .query_row("SELECT used_at FROM phrases WHERE id='used'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "2026-10-23T10:00:00Z"
    );
    assert_eq!(
        copied
            .query_row(
                "SELECT voice_reference FROM phrases WHERE id='used'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "one.wav"
    );
    drop(copied);
    assert!(
        !fs::read(target.join("library.snapshot.sqlite3"))
            .unwrap()
            .windows(b"fixture-secret".len())
            .any(|w| w == b"fixture-secret")
    );
    assert!(!target.join("capture/settings.json").exists());
    for asset in before
        .files
        .iter()
        .filter(|a| !DB_FILES.contains(&a.path.as_str()) && a.path != "settings.json")
    {
        assert_eq!(
            hash(&target.join(&asset.path), &cancel()).unwrap(),
            (asset.bytes, asset.sha256.clone())
        );
    }
}

#[test]
fn repeat_import_does_not_duplicate_rows_or_change_originals() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("preview");
    let before = inventory(&source, &cancel()).unwrap();
    let first = run(&source, &target).unwrap();
    let second = run(&source, &target).unwrap();
    assert_eq!(first.rows, second.rows);
    assert_eq!(first.model_roots, second.model_roots);
    assert_eq!(inventory(&source, &cancel()).unwrap(), before);
    let db = Connection::open(target.join("library.snapshot.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM phrases", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn absent_backend_row_uses_effective_legacy_defaults_before_migration() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("preview");
    let mut settings = read_json(&source.join("settings.json")).unwrap();
    settings["BackendSettingsOverrides"] = json!([]);
    settings["TextModel"] = json!("text-saiga-llama3-8b");
    fs::write(
        source.join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let db = Connection::open(source.join("library.sqlite3")).unwrap();
    db.execute("DELETE FROM settings WHERE key='text_model'", [])
        .unwrap();
    drop(db);
    run(&source, &target).unwrap();
    let db = Connection::open(target.join("library.snapshot.sqlite3")).unwrap();
    let result: SettingsSnapshot = serde_json::from_str(
        &db.query_row("SELECT json FROM app_settings", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result.desktop["TextModel"], "text-gemma4-12b");
}

#[test]
fn cancellation_after_capture_is_resumable_and_not_ready() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("preview");
    let flag = cancel();
    let result = prepare_steps(&source, &target, 2_000_000_000, &flag, |phase| {
        if phase == Phase::Copying {
            flag.store(true, Ordering::Relaxed);
        }
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    let state: Journal =
        serde_json::from_value(read_json(&target.join("preview-journal.json")).unwrap()).unwrap();
    assert_eq!(state.phase, Phase::Copying);
    assert!(!target.join("preview-report.json").exists());
    assert_eq!(run(&source, &target).unwrap().phase, Phase::Ready);
}

#[test]
fn changed_source_cannot_be_mixed_into_an_existing_preview() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("preview");
    run(&source, &target).unwrap();
    fs::write(source.join("audio/ready.flac"), b"changed audio").unwrap();
    assert!(matches!(run(&source, &target), Err(Error::SourceChanged)));
    assert_ne!(
        fs::read(source.join("audio/ready.flac")).unwrap(),
        fs::read(target.join("audio/ready.flac")).unwrap()
    );
}

#[test]
fn unrelated_destination_limit_and_nested_target_fail_safely() {
    let root = temp();
    let source = fixture(root.path());
    let unrelated = root.path().join("my-files");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep.txt"), b"keep").unwrap();
    assert!(run(&source, &unrelated).is_err());
    assert_eq!(fs::read(unrelated.join("keep.txt")).unwrap(), b"keep");
    let limited = root.path().join("limited");
    assert!(prepare_steps(&source, &limited, 1, &cancel(), |_| {}).is_err());
    assert!(!limited.exists());
    assert!(run(&source, &source.join("nested")).is_err());
    assert!(!source.join("nested").exists());
}

#[test]
fn corrupt_database_and_escaping_audio_never_produce_ready() {
    let root = temp();
    let source = fixture(root.path());
    let db = Connection::open(source.join("library.sqlite3")).unwrap();
    db.execute(
        "UPDATE phrases SET audio_path='../outside.flac' WHERE id='used'",
        [],
    )
    .unwrap();
    drop(db);
    let target = root.path().join("bad-path");
    assert!(run(&source, &target).is_err());
    assert!(!target.join("preview-report.json").exists());
    fs::write(source.join("library.sqlite3"), b"not a database").unwrap();
    let target = root.path().join("bad-db");
    assert!(run(&source, &target).is_err());
    assert!(!target.join("preview-report.json").exists());
}

#[test]
fn missing_live_audio_is_not_reported_as_a_successful_import() {
    let root = temp();
    let source = fixture(root.path());
    fs::remove_file(source.join("audio/ready.flac")).unwrap();
    let target = root.path().join("missing-audio");
    assert!(run(&source, &target).is_err());
    assert!(!target.join("preview-report.json").exists());
}

#[test]
fn concurrent_preview_writer_is_rejected_without_changing_ready_data() {
    let root = temp();
    let source = fixture(root.path());
    let target = root.path().join("preview");
    run(&source, &target).unwrap();
    let before = hash(&target.join("library.snapshot.sqlite3"), &cancel()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(target.join(".preview.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(matches!(run(&source, &target), Err(Error::Busy)));
    assert_eq!(
        hash(&target.join("library.snapshot.sqlite3"), &cancel()).unwrap(),
        before
    );
}

#[test]
fn database_wal_is_included_without_opening_original_with_sqlite() {
    let root = temp();
    let source = fixture(root.path());
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "preview::tests::wal_fixture_child",
            "--nocapture",
        ])
        .env("KAVYCHKA_TEST_WAL_SOURCE", &source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(source.join("library.sqlite3-wal").is_file());
    let before = inventory(&source, &cancel()).unwrap();
    let target = root.path().join("wal-preview");
    assert_eq!(run(&source, &target).unwrap().rows["phrases"], 3);
    let db = Connection::open(target.join("library.snapshot.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT text FROM phrases WHERE id='wal-only'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "Committed only in WAL"
    );
    assert_eq!(inventory(&source, &cancel()).unwrap(), before);
}

#[test]
#[ignore = "fixture subprocess invoked explicitly by the WAL preservation test"]
fn wal_fixture_child() {
    let source = PathBuf::from(std::env::var_os("KAVYCHKA_TEST_WAL_SOURCE").expect("fixture only"));
    assert!(source.join(".test-fixture").is_file());
    let db = Connection::open(source.join("library.sqlite3")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        INSERT INTO phrases(id,text,normalized,category,model,profile,created_at)
        VALUES('wal-only','Committed only in WAL','wal-only','break','builtin','default','2026-10-23');").unwrap();
    // Exit without SQLite close, leaving committed WAL like an abrupt shutdown.
    std::mem::forget(db);
    std::process::exit(0);
}
