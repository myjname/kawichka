use super::*;
use crate::{datafs, library::Phrase, presets, sandbox::Sandbox};
use kavychka_core::preferences::{Preferences, Theme};
use std::{
    fs,
    io::{Read, Write},
    sync::atomic::Ordering,
};
fn temp() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/p5-storage-tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
pub(super) fn wave(value: i16) -> Vec<u8> {
    let samples = 16000u32;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((36 + samples * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((samples * 2).to_le_bytes());
    for _ in 0..samples {
        b.extend(value.to_le_bytes());
    }
    b
}
pub(super) fn row(id: &str) -> Phrase {
    Phrase {
        id: id.into(),
        text: format!("Синтетическая фраза {id} для проверки сохранности."),
        category: "exercise".into(),
        model: "system".into(),
        profile: "system".into(),
        day: Some("2026-10-23".into()),
        audio_path: None,
        approved: false,
        deleted: false,
        rejected: false,
        used_at: None,
        created_at: "2026-10-23T09:00:00Z".into(),
        followup: false,
        language: "ru".into(),
        seed: 184,
        voice_style: "calm".into(),
        voice_instruction: "Calmly".into(),
        voice_reference: String::new(),
    }
}
pub(super) fn source(root: &Path) -> (Sandbox, String) {
    let mut source = Sandbox::open(&root.join("source")).unwrap();
    let character = presets::save_character(
        &source.root,
        "",
        "Fixture character".into(),
        "Короткий нейтральный характер.".into(),
    )
    .unwrap();
    let mut prefs = Preferences {
        user_name: "Fixture owner".into(),
        theme: Theme::Dark,
        ..Default::default()
    };
    prefs.desktop.setup_complete = true;
    prefs.desktop.character_preset = character.clone();
    prefs.generation.auto_daily = false;
    prefs.generation.model_paths.insert(
        "text-gemma4-12b".into(),
        root.join("external/model.gguf")
            .to_string_lossy()
            .into_owned(),
    );
    source.save(&prefs).unwrap();
    let staging = root.join("staging");
    fs::create_dir_all(staging.join("audio")).unwrap();
    fs::write(staging.join("audio/recording.wav"), wave(12)).unwrap();
    let mut first = row("same");
    first.approved = true;
    first.used_at = Some("2026-10-23T10:00:00Z".into());
    first.audio_path = Some("recording.wav".into());
    first.profile = character.clone();
    source.accept_phrase(&first, Some(&staging)).unwrap();
    let mut second = row("voice-new");
    second.audio_path = Some("recording.wav".into());
    second.profile = character.clone();
    source.accept_phrase(&second, Some(&staging)).unwrap();
    let mut third = row("excluded");
    third.rejected = true;
    source.accept_phrase(&third, None).unwrap();
    let mut fourth = row("history");
    fourth.deleted = true;
    fourth.used_at = Some("2026-10-22T10:00:00Z".into());
    source.accept_phrase(&fourth, None).unwrap();
    source
        .put_meta("voice_rotation", &serde_json::json!(3))
        .unwrap();
    source.put_meta("origin",&serde_json::json!({"source":root.join("external"),"snapshot":source.root.join("legacy"),"imported_rows":4})).unwrap();
    source
        .connection
        .execute(
            "INSERT INTO delivery_claims VALUES('old-notice','same')",
            [],
        )
        .unwrap();
    let mut state = source.load_state().unwrap();
    state
        .handled
        .insert("old-event".into(), "2026-10-23T10:00:00Z".parse().unwrap());
    source.save_runtime(&prefs, &state).unwrap();
    (source, character)
}
pub(super) fn exported(source: &Sandbox, root: &Path, options: ExportOptions) -> PathBuf {
    let output = root.join(format!("{}.zip", datafs::id()));
    CaptureSource::new(source)
        .unwrap()
        .export(
            &output,
            &root.join(datafs::id()),
            options,
            "0.1.1",
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
    output
}
fn inspected(archive: &Path, root: &Path) -> Inspected {
    inspect(
        archive,
        &root.join(datafs::id()),
        2_000_000_000,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}
pub(super) fn tree_hash(root: &Path) -> String {
    let cancel = AtomicBool::new(false);
    let entries: Vec<_> = datafs::tree(root, &cancel)
        .unwrap()
        .into_iter()
        .filter(|p| p != "shell.lock")
        .map(|p| {
            let h = datafs::hash(&root.join(&p), &cancel).unwrap();
            (p, h)
        })
        .collect();
    format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&entries).unwrap())
    )
}
use sha2::Digest;
#[test]
fn encouragement_metadata_survives_full_profile_and_is_omitted_without_history() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let mut support = crate::support::load(&source, &source.load().unwrap(), "2026-10-28").unwrap();
    crate::support::accept(
        &source,
        &mut support,
        vec![
            "A small step still counts today.".into(),
            "There is room for a second try.".into(),
        ],
    )
    .unwrap();
    let archive = exported(&source, dir.path(), ExportOptions::default());
    let inspected = inspected(&archive, dir.path());
    assert_eq!(
        inspected.snapshot.meta.get("support_history"),
        source.meta("support_history").unwrap().as_ref()
    );
    let mut target = Sandbox::open(&dir.path().join("support-target")).unwrap();
    let mut plan = RestorePlan::prepare(&target, inspected, &AtomicBool::new(false)).unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    plan.apply(&mut target, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        source.meta("support_history").unwrap(),
        target.meta("support_history").unwrap()
    );
    let archive = exported(
        &source,
        dir.path(),
        ExportOptions {
            library: false,
            audio: false,
        },
    );
    let checked = super::archive::inspect(
        &archive,
        &dir.path().join("support-no-history"),
        2_000_000_000,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(!checked.snapshot.meta.contains_key("support_day"));
    assert!(!checked.snapshot.meta.contains_key("support_history"));
}
#[test]
fn full_profile_restores_settings_history_presets_audio_and_claims_without_touching_source() {
    let dir = temp();
    let (source, character) = source(dir.path());
    let before = tree_hash(&source.root);
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("target")).unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(plan.summary().clean_profile);
    assert_eq!(plan.summary().phrases_added, 4);
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let result = plan.apply(&mut target, &AtomicBool::new(false)).unwrap();
    assert!(result.warning.is_none());
    assert_eq!(
        serde_json::to_value(source.load().unwrap()).unwrap(),
        serde_json::to_value(target.load().unwrap()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(source.load_state().unwrap()).unwrap(),
        serde_json::to_value(target.load_state().unwrap()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(source.phrases().unwrap()).unwrap(),
        serde_json::to_value(target.phrases().unwrap()).unwrap()
    );
    assert_eq!(
        fs::read(target.root.join("audio/recording.wav")).unwrap(),
        wave(12)
    );
    assert_eq!(
        presets::read(&target.root, &character).unwrap().name,
        "Fixture character"
    );
    assert!(target.origin().unwrap().is_none());
    assert!(target.meta("profile_origin_hint").unwrap().is_some());
    assert_eq!(
        target.meta("voice_rotation").unwrap(),
        Some(serde_json::json!(3))
    );
    assert_eq!(
        target
            .connection
            .query_row::<String, _, _>(
                "SELECT phrase_id FROM delivery_claims WHERE notice='old-notice'",
                [],
                |r| r.get(0)
            )
            .unwrap(),
        "same"
    );
    assert_eq!(before, tree_hash(&source.root));
    let old_files = tree_hash(&target.root);
    let again = RestorePlan::prepare(
        &target,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(again.summary().already_imported);
    assert!(
        again
            .apply(&mut target, &AtomicBool::new(false))
            .unwrap()
            .already_imported
    );
    assert_eq!(old_files, tree_hash(&target.root));
}

#[test]
fn complete_character_keeps_all_components_through_profile_backup_and_restore() {
    let dir = temp();
    let (mut source, _) = source(dir.path());
    let media = dir.path().join("pack-media");
    fs::create_dir(&media).unwrap();
    fs::write(media.join("reference.wav"), wave(3)).unwrap();
    fs::write(media.join("pet.gif"), b"GIF89a prepared metadata fixture").unwrap();
    let voice = presets::import_prepared(
        &source.root,
        &presets::Preset {
            version: 1,
            name: "Voice fixture".into(),
            content: presets::Content::Voice {
                references: vec![presets::Reference {
                    file: "reference.wav".into(),
                    transcript: "Prepared reference.".into(),
                }],
            },
        },
        Some(&media),
    )
    .unwrap();
    let visual = presets::import_prepared(
        &source.root,
        &presets::Preset {
            version: 1,
            name: "Appearance fixture".into(),
            content: presets::Content::Visual {
                file: "pet.gif".into(),
                layout: "pixel".into(),
            },
        },
        Some(&media),
    )
    .unwrap();
    let mut p = source.load().unwrap();
    p.generation.voice_profile = voice;
    p.desktop.visual_preset = visual;
    p.generation.character_description = "Portable complete character.".into();
    let pack = presets::save_current_pack(&source.root, "Complete fixture".into(), &p).unwrap();
    p.desktop.character_preset = pack.clone();
    p.generation.voice_profile = pack.clone();
    p.desktop.visual_preset = pack.clone();
    source.save(&p).unwrap();
    let original = presets::read(&source.root, &pack).unwrap();
    let before = tree_hash(&source.root);
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("target")).unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.31",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    plan.apply(&mut target, &AtomicBool::new(false)).unwrap();
    let restored = target.load().unwrap();
    assert_eq!(restored.desktop.character_preset, pack);
    assert_eq!(restored.generation.voice_profile, pack);
    assert_eq!(restored.desktop.visual_preset, pack);
    assert_eq!(
        serde_json::to_value(p).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
    let actual = presets::read(&target.root, &pack).unwrap();
    assert_eq!(
        serde_json::to_value(&original).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    for file in original.files() {
        assert_eq!(
            fs::read(presets::folder(&source.root, &pack).unwrap().join(file)).unwrap(),
            fs::read(presets::folder(&target.root, &pack).unwrap().join(file)).unwrap()
        );
    }
    assert_eq!(before, tree_hash(&source.root));
    // A different local character using the same ID must keep its settings;
    // the incoming self-contained package is remapped without broken links.
    let mut occupied = Sandbox::open(&dir.path().join("occupied")).unwrap();
    let destination = presets::folder(&occupied.root, &pack).unwrap();
    fs::create_dir(&destination).unwrap();
    for file in original.files() {
        fs::copy(
            presets::folder(&source.root, &pack).unwrap().join(file),
            destination.join(file),
        )
        .unwrap();
    }
    fs::copy(
        presets::folder(&source.root, &pack)
            .unwrap()
            .join("preset.json"),
        destination.join("preset.json"),
    )
    .unwrap();
    let mut local = presets::read(&occupied.root, &pack).unwrap();
    local.name = "Local character kept".into();
    presets::update(&occupied.root, &pack, &local).unwrap();
    let mut chosen = occupied.load().unwrap();
    chosen.user_name = "Existing profile".into();
    chosen.desktop.character_preset = pack.clone();
    chosen.generation.voice_profile = pack.clone();
    chosen.desktop.visual_preset = pack.clone();
    occupied.save(&chosen).unwrap();
    let wanted = serde_json::to_value(chosen).unwrap();
    let mut plan = RestorePlan::prepare(
        &occupied,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!plan.summary().clean_profile);
    plan.prepare_files(
        CaptureSource::new(&occupied).unwrap(),
        "0.1.31",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    plan.apply(&mut occupied, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        serde_json::to_value(occupied.load().unwrap()).unwrap(),
        wanted
    );
    assert_eq!(
        presets::read(&occupied.root, &pack).unwrap().name,
        "Local character kept"
    );
    let imported = presets::list(&occupied.root)
        .unwrap()
        .into_iter()
        .find(|e| e.id != pack && e.preset.name == original.name)
        .unwrap();
    for file in imported.preset.files() {
        assert_eq!(
            fs::read(presets::folder(&source.root, &pack).unwrap().join(file)).unwrap(),
            fs::read(
                presets::folder(&occupied.root, &imported.id)
                    .unwrap()
                    .join(file)
            )
            .unwrap()
        );
    }
}

#[test]
fn automatic_first_launch_checkpoint_is_still_a_clean_profile_but_user_changes_are_not() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("started-target")).unwrap();
    let prefs = target.load().unwrap();
    let mut state = target.load_state().unwrap();
    state.checkpoint_utc = Some("2026-10-25T12:00:00Z".parse().unwrap());
    target.save_runtime(&prefs, &state).unwrap();
    let cancel = AtomicBool::new(false);
    let mut plan = RestorePlan::prepare(&target, inspected(&zip, dir.path()), &cancel).unwrap();
    assert!(
        plan.summary().clean_profile,
        "automatic startup must not force merge and discard archive settings"
    );
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.25",
        &cancel,
        |_| {},
    )
    .unwrap();
    plan.apply(&mut target, &cancel).unwrap();
    assert_eq!(
        target.load().unwrap().user_name,
        source.load().unwrap().user_name
    );
    let mut changed = Sandbox::open(&dir.path().join("changed-defaults")).unwrap();
    let mut p = changed.load().unwrap();
    p.user_name = "Keep my current settings".into();
    changed.save(&p).unwrap();
    let plan = RestorePlan::prepare(&changed, inspected(&zip, dir.path()), &cancel).unwrap();
    assert!(!plan.summary().clean_profile);
}

#[test]
fn restore_plan_rejects_settings_changed_after_clean_review() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("stale-settings")).unwrap();
    let cancel = AtomicBool::new(false);
    target
        .save_runtime(&Preferences::default(), &Default::default())
        .unwrap();
    let mut plan = RestorePlan::prepare(&target, inspected(&zip, dir.path()), &cancel).unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.25",
        &cancel,
        |_| {},
    )
    .unwrap();
    let mut p = target.load().unwrap();
    p.user_name = "Changed after review".into();
    target.save(&p).unwrap();
    assert!(
        plan.apply(&mut target, &cancel).is_err(),
        "restore overwrote preferences changed after review"
    );
    assert_eq!(target.load().unwrap().user_name, "Changed after review");
    assert!(target.phrases().unwrap().is_empty());
}

#[test]
fn periodic_checkpoint_does_not_invalidate_an_otherwise_unchanged_restore() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("periodic-target")).unwrap();
    let cancel = AtomicBool::new(false);
    let mut plan = RestorePlan::prepare(&target, inspected(&zip, dir.path()), &cancel).unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.25",
        &cancel,
        |_| {},
    )
    .unwrap();
    let mut state = target.load_state().unwrap();
    state.checkpoint_utc = Some("2026-10-25T12:00:05Z".parse().unwrap());
    target
        .save_runtime(&target.load().unwrap(), &state)
        .unwrap();
    plan.apply(&mut target, &cancel).unwrap();
    assert_eq!(
        target.load().unwrap().user_name,
        source.load().unwrap().user_name
    );
}

#[test]
fn clean_restore_preserves_custom_recovery_budget_but_never_overwrites_existing_state() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let cancel = AtomicBool::new(false);
    let mut target = Sandbox::open(&dir.path().join("custom-budget")).unwrap();
    let mut prefs = target.load().unwrap();
    prefs.desktop.recovery_limit_bytes = 3_000_000_000;
    target.save_runtime(&prefs, &Default::default()).unwrap();
    let mut plan = RestorePlan::prepare(&target, inspected(&zip, dir.path()), &cancel).unwrap();
    assert!(plan.summary().clean_profile);
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.25",
        &cancel,
        |_| {},
    )
    .unwrap();
    plan.apply(&mut target, &cancel).unwrap();
    assert_eq!(
        target.load().unwrap().user_name,
        source.load().unwrap().user_name
    );
    assert_eq!(
        target.load().unwrap().desktop.recovery_limit_bytes,
        3_000_000_000
    );
    let target = Sandbox::open(&dir.path().join("state-without-preferences")).unwrap();
    let state = kavychka_core::engine::State {
        pause_forever: true,
        ..Default::default()
    };
    target
        .connection
        .execute(
            "INSERT INTO reminder_state VALUES(1,?1)",
            [serde_json::to_string(&state).unwrap()],
        )
        .unwrap();
    let plan = RestorePlan::prepare(&target, inspected(&zip, dir.path()), &cancel).unwrap();
    assert!(
        !plan.summary().clean_profile,
        "an absent preferences row does not make existing reminder state disposable"
    );
}
fn populated(root: &Path, character: &str) -> Sandbox {
    let mut target = Sandbox::open(&root.join("target")).unwrap();
    let folder = presets::folder(&target.root, character).unwrap();
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join("preset.json"),
        serde_json::to_vec(&presets::Preset {
            version: 1,
            name: "Current character".into(),
            content: presets::Content::Character {
                description: "Сохранить нынешний характер.".into(),
            },
        })
        .unwrap(),
    )
    .unwrap();
    let mut p = Preferences::default();
    p.desktop.setup_complete = true;
    p.user_name = "Current user".into();
    p.desktop.character_preset = character.into();
    target.save(&p).unwrap();
    fs::write(target.root.join("audio/recording.wav"), wave(99)).unwrap();
    let mut existing = row("same");
    existing.text = "Текущий исключённый текст побеждает.".into();
    existing.rejected = true;
    target.accept_phrase(&existing, None).unwrap();
    target
}
#[test]
fn nonempty_restore_keeps_current_exclusions_and_settings_and_maps_conflicting_media_and_presets() {
    let dir = temp();
    let (source, character) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = populated(dir.path(), &character);
    let prefs = serde_json::to_value(target.load().unwrap()).unwrap();
    let original = serde_json::to_value(target.phrase("same").unwrap()).unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!plan.summary().clean_profile);
    assert_eq!(plan.summary().phrases_kept, 1);
    assert_eq!(plan.summary().presets_added, 1);
    assert!(plan.summary().conflict_count >= 3);
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    plan.apply(&mut target, &AtomicBool::new(false)).unwrap();
    assert_eq!(prefs, serde_json::to_value(target.load().unwrap()).unwrap());
    assert_eq!(
        original,
        serde_json::to_value(target.phrase("same").unwrap()).unwrap()
    );
    assert_eq!(
        fs::read(target.root.join("audio/recording.wav")).unwrap(),
        wave(99)
    );
    let new = target.phrase("voice-new").unwrap();
    assert_ne!(new.profile, character);
    assert_ne!(new.audio_path.as_deref(), Some("recording.wav"));
    assert!(new.used_at.is_none() && !new.approved);
    assert_eq!(
        fs::read(target.audio_file(&new).unwrap().unwrap()).unwrap(),
        wave(12)
    );
    assert_eq!(
        presets::read(&target.root, &new.profile).unwrap().name,
        "Fixture character"
    );
    assert!(result_has_backup(&target));
    let reexport = exported(&target, dir.path(), ExportOptions::default());
    let reread = inspected(&reexport, dir.path());
    let sha = datafs::hash(&zip, &AtomicBool::new(false)).unwrap().1;
    assert!(
        reread
            .root
            .join(format!("legacy/profiles/{sha}/library.sqlite3"))
            .is_file()
    );
    assert!(
        reread
            .snapshot
            .meta
            .contains_key(&format!("profile_history:{sha}"))
    );
}
fn result_has_backup(target: &Sandbox) -> bool {
    fs::read_dir(target.root.join("recovery/profiles"))
        .unwrap()
        .any(|d| d.unwrap().path().join("before.zip").is_file())
}
#[test]
fn failed_commit_removes_only_its_new_files_and_retains_current_data_and_backup() {
    let dir = temp();
    let (source, character) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    let mut target = populated(dir.path(), &character);
    let before_rows = serde_json::to_value(target.phrases().unwrap()).unwrap();
    let before_preset = tree_hash(&target.root.join("presets"));
    let before_audio = tree_hash(&target.root.join("audio"));
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&zip, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    target.connection.execute_batch("CREATE TRIGGER fail_restore BEFORE INSERT ON phrases BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(plan.apply(&mut target, &AtomicBool::new(false)).is_err());
    assert_eq!(
        before_rows,
        serde_json::to_value(target.phrases().unwrap()).unwrap()
    );
    assert_eq!(before_preset, tree_hash(&target.root.join("presets")));
    assert_eq!(before_audio, tree_hash(&target.root.join("audio")));
    assert!(!target.root.join("profile-restore.json").exists());
    assert!(result_has_backup(&target));
}
#[test]
fn composition_excludes_unselected_history_and_recordings_and_cancellation_never_publishes_an_archive()
 {
    let dir = temp();
    let (source, _) = source(dir.path());
    let legacy = kavychka_core::legacy::reconcile(
        &serde_json::json!({"History":["Old text history must be excluded"]})
            .as_object()
            .unwrap()
            .clone(),
        &Default::default(),
    )
    .unwrap();
    assert!(legacy.desktop.contains_key("History"));
    source
        .put_meta("legacy_settings", &serde_json::to_value(legacy).unwrap())
        .unwrap();
    let before = tree_hash(&source.root);
    let settings = exported(
        &source,
        dir.path(),
        ExportOptions {
            library: false,
            audio: false,
        },
    );
    let settings_view = inspected(&settings, dir.path());
    assert!(
        settings_view.snapshot.meta["legacy_settings"]["desktop"]
            .get("History")
            .is_none()
    );
    assert!(
        settings_view.snapshot.phrases.is_empty()
            && settings_view.snapshot.claims.is_empty()
            && settings_view.snapshot.state.handled.is_empty()
    );
    assert!(
        settings_view
            .manifest
            .entries
            .iter()
            .all(|e| !e.path.starts_with("audio/") && !e.path.starts_with("legacy/"))
    );
    let texts = exported(
        &source,
        dir.path(),
        ExportOptions {
            library: true,
            audio: false,
        },
    );
    let inspected = inspected(&texts, dir.path());
    assert_eq!(inspected.snapshot.phrases.len(), 4);
    assert!(
        inspected
            .snapshot
            .phrases
            .iter()
            .all(|r| r.audio_path.is_none())
    );
    let cancel = AtomicBool::new(false);
    let output = dir.path().join("cancelled.zip");
    assert!(matches!(
        CaptureSource::new(&source).unwrap().export(
            &output,
            &dir.path().join("cancel-job"),
            ExportOptions::default(),
            "0.1.1",
            &cancel,
            |_| {
                cancel.store(true, Ordering::Release);
            }
        ),
        Err(Error::Cancelled)
    ));
    assert!(!output.exists());
    let cancel = AtomicBool::new(false);
    let output = dir.path().join("cancelled-during-zip.zip");
    assert!(matches!(
        CaptureSource::new(&source).unwrap().export(
            &output,
            &dir.path().join("cancel-zip-job"),
            ExportOptions::default(),
            "0.1.1",
            &cancel,
            |stage| {
                if stage == "archive" {
                    cancel.store(true, Ordering::Release);
                }
            }
        ),
        Err(Error::Cancelled)
    ));
    assert!(!output.exists());
    assert!(
        fs::read_dir(dir.path()).unwrap().all(|p| p
            .unwrap()
            .path()
            .extension()
            .is_none_or(|s| s != "part"))
    );
    assert_eq!(before, tree_hash(&source.root));
}
fn rewrite_zip(source: &Path, dest: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut old = zip::ZipArchive::new(fs::File::open(source).unwrap()).unwrap();
    let mut entries = vec![];
    for i in 0..old.len() {
        let mut f = old.by_index(i).unwrap();
        let mut bytes = vec![];
        f.read_to_end(&mut bytes).unwrap();
        entries.push((f.name().to_owned(), bytes));
    }
    let manifest = entries
        .iter_mut()
        .find(|(n, _)| n == "manifest.json")
        .unwrap();
    let mut value = serde_json::from_slice(&manifest.1).unwrap();
    edit(&mut value);
    manifest.1 = serde_json::to_vec(&value).unwrap();
    let mut z = zip::ZipWriter::new(fs::File::create(dest).unwrap());
    for (name, bytes) in entries {
        z.start_file(name, zip::write::FileOptions::default())
            .unwrap();
        z.write_all(&bytes).unwrap();
    }
    z.finish().unwrap();
}
#[test]
fn character_archives_are_identified_before_inspection_job_or_profile_changes() {
    let dir = temp();
    let (current, _) = source(dir.path());
    let before = tree_hash(&current.root);
    let original_prefs = serde_json::to_value(current.load().unwrap()).unwrap();
    let original_rows = serde_json::to_value(current.phrases().unwrap()).unwrap();
    for (case, entry) in [
        ("single", "preset.json"),
        ("collection", "parts/component.KAVYCHKA"),
    ] {
        let archive = dir.path().join(format!("character-{case}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file(entry, zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"not extracted or interpreted here").unwrap();
        zip.finish().unwrap();
        let job = current.root.join("jobs").join(format!("inspect-{case}"));
        let result = inspect(
            &archive,
            &job,
            2_000_000_000,
            &AtomicBool::new(false),
            |_| panic!("wrong archive must not reach extraction"),
        );
        assert!(
            matches!(result, Err(Error::Invalid(message)) if message == "character preset archive; import in Character after setup")
        );
        assert!(!job.exists());
        assert_eq!(tree_hash(&current.root), before);
    }
    assert_eq!(
        serde_json::to_value(current.load().unwrap()).unwrap(),
        original_prefs
    );
    assert_eq!(
        serde_json::to_value(current.phrases().unwrap()).unwrap(),
        original_rows
    );
}

#[test]
fn wrong_hash_future_format_and_traversal_are_rejected_before_any_target_profile_change() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let zip = exported(&source, dir.path(), ExportOptions::default());
    for case in ["hash", "future", "traversal"] {
        let archive = dir.path().join(format!("{case}.zip"));
        rewrite_zip(&zip, &archive, |manifest| match case {
            "hash" => manifest["entries"][0]["sha256"] = serde_json::json!("0".repeat(64)),
            "future" => manifest["format"] = serde_json::json!(2),
            _ => manifest["entries"][0]["path"] = serde_json::json!("../escape"),
        });
        assert!(
            inspect(
                &archive,
                &dir.path().join(format!("job-{case}")),
                2_000_000_000,
                &AtomicBool::new(false),
                |_| {}
            )
            .is_err()
        );
    }
    assert!(!dir.path().join("escape").exists());
}

#[test]
fn correctly_hashed_archives_with_unknown_sql_schema_or_missing_references_are_rejected() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let original = exported(&source, dir.path(), ExportOptions::default());
    for (case, sql) in [
        ("future-schema", "PRAGMA user_version=4"),
        (
            "trigger",
            "CREATE TRIGGER unknown_profile_code AFTER INSERT ON phrases BEGIN SELECT 1; END",
        ),
        ("reference", "UPDATE phrases SET audio_path='missing.flac'"),
    ] {
        let mut input = zip::ZipArchive::new(fs::File::open(&original).unwrap()).unwrap();
        let mut entries = vec![];
        for i in 0..input.len() {
            let mut f = input.by_index(i).unwrap();
            let mut bytes = vec![];
            f.read_to_end(&mut bytes).unwrap();
            entries.push((f.name().to_owned(), bytes));
        }
        let sqlite = dir.path().join(format!("{case}.sqlite3"));
        let database = entries
            .iter_mut()
            .find(|(n, _)| n == "library.sqlite3")
            .unwrap();
        fs::write(&sqlite, &database.1).unwrap();
        let db = rusqlite::Connection::open(&sqlite).unwrap();
        db.execute_batch(sql).unwrap();
        drop(db);
        database.1 = fs::read(&sqlite).unwrap();
        let size = database.1.len();
        let sha = format!("{:x}", sha2::Sha256::digest(&database.1));
        let manifest = entries
            .iter_mut()
            .find(|(n, _)| n == "manifest.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        let record = value["entries"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["path"] == "library.sqlite3")
            .unwrap();
        record["bytes"] = serde_json::json!(size);
        record["sha256"] = serde_json::json!(sha);
        manifest.1 = serde_json::to_vec(&value).unwrap();
        let archive = dir.path().join(format!("{case}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        for (name, bytes) in entries {
            zip.start_file(name, zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        assert!(
            inspect(
                &archive,
                &dir.path().join(format!("inspect-{case}")),
                2_000_000_000,
                &AtomicBool::new(false),
                |_| {}
            )
            .is_err(),
            "{case}"
        );
    }
}

#[test]
fn sqlite_full_during_restore_rolls_back_database_and_published_files() {
    let dir = temp();
    let (mut source, _) = source(dir.path());
    for i in 0..16 {
        let mut r = row(&format!("large-{i}"));
        r.text = format!("{i} {}", "Ж".repeat(3900));
        source.accept_phrase(&r, None).unwrap();
    }
    let archive = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("full-target")).unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&archive, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let pages: i64 = target
        .connection
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    target
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let error = plan
        .apply(&mut target, &AtomicBool::new(false))
        .unwrap_err();
    assert!(
        matches!(error,Error::Sqlite(rusqlite::Error::SqliteFailure(ref e,_)) if e.code==rusqlite::ErrorCode::DiskFull),
        "{error}"
    );
    assert!(target.phrases().unwrap().is_empty());
    assert!(presets::scan(&target.root).unwrap().0.is_empty());
    assert!(
        fs::read_dir(target.root.join("audio"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(!target.root.join("profile-restore.json").exists());
}

#[test]
fn real_process_crashes_before_after_commit_and_during_recovery_are_idempotent() {
    for stage in [
        "restore_journal",
        "restore_file",
        "restore_before_commit",
        "restore_after_commit",
        "restore_cleanup_file",
        "restore_cleanup_journal",
    ] {
        let dir = temp();
        let (source, _) = source(dir.path());
        let archive = exported(&source, dir.path(), ExportOptions::default());
        drop(source);
        let target = dir.path().join("crash-target");
        drop(Sandbox::open(&target).unwrap());
        let child = |point: &str, recovery: bool| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "profile::tests::profile_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("KAVYCHKA_PROFILE_CRASH_DIR", dir.path())
                .env("KAVYCHKA_PROFILE_CRASH_ARCHIVE", &archive)
                .env("KAVYCHKA_PROFILE_CRASH_STAGE", point)
                .env(
                    "KAVYCHKA_PROFILE_CRASH_RECOVERY",
                    if recovery { "1" } else { "0" },
                )
                .output()
                .unwrap()
        };
        if stage.starts_with("restore_cleanup") {
            assert_eq!(child("restore_file", false).status.code(), Some(77));
            assert_eq!(child(stage, true).status.code(), Some(77), "{stage}");
        } else {
            let output = child(stage, false);
            assert_eq!(
                output.status.code(),
                Some(77),
                "{stage}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let target = Sandbox::open(&target).unwrap();
        assert_eq!(
            target.phrases().unwrap().len(),
            if stage == "restore_after_commit" {
                4
            } else {
                0
            },
            "{stage}"
        );
        assert!(!target.root.join("profile-restore.json").exists());
        if stage == "restore_after_commit" {
            assert!(target.root.join("audio/recording.wav").is_file());
        } else {
            assert!(
                fs::read_dir(target.root.join("audio"))
                    .unwrap()
                    .next()
                    .is_none()
            );
        }
        let path = target.root.clone();
        drop(target);
        assert!(Sandbox::open(&path).is_ok());
    }
}
#[test]
#[ignore = "child process only, owned synthetic profile fixture"]
fn profile_crash_child() {
    let Some(dir) = std::env::var_os("KAVYCHKA_PROFILE_CRASH_DIR").map(PathBuf::from) else {
        return;
    };
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../work/p5-storage-tests")
        .canonicalize()
        .unwrap();
    assert!(dir.canonicalize().unwrap().starts_with(expected));
    let stage = std::env::var("KAVYCHKA_PROFILE_CRASH_STAGE").unwrap();
    if std::env::var("KAVYCHKA_PROFILE_CRASH_RECOVERY").unwrap() == "1" {
        super::FAULT.with(|f| *f.borrow_mut() = stage);
        Sandbox::open(&dir.join("crash-target")).unwrap();
    } else {
        let archive = PathBuf::from(std::env::var_os("KAVYCHKA_PROFILE_CRASH_ARCHIVE").unwrap());
        let mut target = Sandbox::open(&dir.join("crash-target")).unwrap();
        let mut plan =
            RestorePlan::prepare(&target, inspected(&archive, &dir), &AtomicBool::new(false))
                .unwrap();
        plan.prepare_files(
            CaptureSource::new(&target).unwrap(),
            "0.1.1",
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        super::FAULT.with(|f| *f.borrow_mut() = stage);
        plan.apply(&mut target, &AtomicBool::new(false)).unwrap();
    }
    panic!("fault boundary not reached");
}

#[test]
fn target_changed_since_review_and_budget_overflow_never_publish_or_drop_existing_data() {
    let dir = temp();
    let (source, _) = source(dir.path());
    let archive = exported(&source, dir.path(), ExportOptions::default());
    let mut target = Sandbox::open(&dir.path().join("changed-target")).unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&archive, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    target
        .accept_phrase(&row("user-edited-after-review"), None)
        .unwrap();
    assert!(plan.apply(&mut target, &AtomicBool::new(false)).is_err());
    assert_eq!(target.phrases().unwrap().len(), 1);
    assert!(
        fs::read_dir(target.root.join("audio"))
            .unwrap()
            .next()
            .is_none()
    );
    target.set_recovery_limit(100_000_000).unwrap();
    fs::create_dir_all(target.root.join("quarantine/quota-fixture")).unwrap();
    let reserve = target.root.join("quarantine/quota-fixture/keep.bin");
    fs::File::create(&reserve)
        .unwrap()
        .set_len(99_000_000)
        .unwrap();
    let mut plan = RestorePlan::prepare(
        &target,
        inspected(&archive, dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        plan.prepare_files(
            CaptureSource::new(&target).unwrap(),
            "0.1.1",
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert!(reserve.is_file());
    assert_eq!(target.phrases().unwrap().len(), 1);
}

#[cfg(windows)]
#[test]
fn real_ntfs_read_denial_fails_without_publishing_and_acl_is_restored() {
    use std::process::Command;
    let dir = temp();
    let (source, _) = source(dir.path());
    let before = tree_hash(&source.root);
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let who = Command::new(system.join("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    assert!(who.status.success());
    let text = String::from_utf8_lossy(&who.stdout);
    let sid = text
        .split('"')
        .find(|s| s.starts_with("S-1-"))
        .unwrap()
        .to_owned();
    let path = source.root.join("audio/recording.wav");
    struct RestoreAcl {
        tool: PathBuf,
        path: PathBuf,
        sid: String,
    }
    impl Drop for RestoreAcl {
        fn drop(&mut self) {
            let _ = Command::new(&self.tool)
                .arg(&self.path)
                .arg("/remove:d")
                .arg(format!("*{}", self.sid))
                .output();
        }
    }
    let restore = RestoreAcl {
        tool: system.join("icacls.exe"),
        path: path.clone(),
        sid: sid.clone(),
    };
    let deny = Command::new(&restore.tool)
        .arg(&path)
        .arg("/deny")
        .arg(format!("*{sid}:(R)"))
        .output()
        .unwrap();
    assert!(deny.status.success());
    assert_eq!(
        fs::read(&path).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let archive = dir.path().join("denied.zip");
    let result = CaptureSource::new(&source).unwrap().export(
        &archive,
        &dir.path().join("denied-job"),
        ExportOptions::default(),
        "0.1.1",
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(
        matches!(result,Err(Error::Io(ref e)) if e.kind()==std::io::ErrorKind::PermissionDenied)
    );
    assert!(!archive.exists());
    drop(restore);
    assert_eq!(before, tree_hash(&source.root));
}

#[test]
#[ignore = "explicit producer/private-copy matrix under work; never opens the live library"]
fn captured_legacy_profile_roundtrip_preserves_archival_tables_active_rows_and_source() {
    let source =
        PathBuf::from(std::env::var_os("KAVYCHKA_P5_LEGACY_SOURCE").expect("private source path"));
    let output =
        PathBuf::from(std::env::var_os("KAVYCHKA_P5_LEGACY_OUTPUT").expect("new evidence path"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../work")
        .canonicalize()
        .unwrap();
    assert!(source.canonicalize().unwrap().starts_with(&workspace));
    assert!(!output.exists());
    assert!(
        output
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(&workspace)
    );
    fs::create_dir_all(&output).unwrap();
    let cancel = AtomicBool::new(false);
    let before = crate::preview::inventory(&source, &cancel).unwrap();
    let mut working = Sandbox::open(&output.join("working")).unwrap();
    let origin = working.bootstrap_legacy(&source, &cancel).unwrap();
    let settings: kavychka_core::legacy::SettingsSnapshot =
        serde_json::from_value(working.meta("legacy_settings").unwrap().unwrap()).unwrap();
    let prefs = kavychka_core::legacy_active::preferences(&settings).unwrap();
    working.save(&prefs).unwrap();
    fn dump(file: &Path) -> serde_json::Value {
        let db =
            rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let mut result = serde_json::Map::new();
        for table in [
            "phrases",
            "plans",
            "preparation",
            "voice_rotation",
            "app_settings",
            "settings",
        ] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            if !exists {
                continue;
            }
            let mut query = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = query.column_count();
            let rows = query
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|i| match row.get_ref(i).unwrap() {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                            rusqlite::types::ValueRef::Real(n) => serde_json::json!(n),
                            rusqlite::types::ValueRef::Text(s) => {
                                serde_json::json!(String::from_utf8_lossy(s))
                            }
                            rusqlite::types::ValueRef::Blob(b) => serde_json::json!(b),
                        })
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            result.insert(table.into(), serde_json::json!(rows));
        }
        result.into()
    }
    let archival = dump(&origin.snapshot.join("library.snapshot.sqlite3"));
    let archive = exported(&working, &output, ExportOptions::default());
    let mut restored = Sandbox::open(&output.join("restored")).unwrap();
    let mut plan = RestorePlan::prepare(&restored, inspected(&archive, &output), &cancel).unwrap();
    plan.prepare_files(
        CaptureSource::new(&restored).unwrap(),
        "0.1.1",
        &cancel,
        |_| {},
    )
    .unwrap();
    plan.apply(&mut restored, &cancel).unwrap();
    assert_eq!(
        serde_json::to_value(working.load().unwrap()).unwrap(),
        serde_json::to_value(restored.load().unwrap()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(working.phrases().unwrap()).unwrap(),
        serde_json::to_value(restored.phrases().unwrap()).unwrap()
    );
    assert_eq!(
        dump(&restored.root.join("legacy/library.snapshot.sqlite3")),
        archival
    );
    assert_eq!(crate::preview::inventory(&source, &cancel).unwrap(), before);
    let repeat = RestorePlan::prepare(&restored, inspected(&archive, &output), &cancel).unwrap();
    assert!(repeat.summary().already_imported);
    let reexport = exported(&restored, &output, ExportOptions::default());
    let reread = inspected(&reexport, &output);
    assert_eq!(
        dump(&reread.root.join("legacy/library.snapshot.sqlite3")),
        archival
    );
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"source_unchanged":true,"raw_tables_values_preserved":true,"active_rows_settings_equal":true,"repeat_noop":true,"reexport_preserves_legacy":true,"working_rows":working.phrases().unwrap().len(),"archival_phrase_rows":archival["phrases"].as_array().map(Vec::len),"archive_sha256":datafs::hash(&archive,&cancel).unwrap().1})).unwrap()).unwrap();
}
