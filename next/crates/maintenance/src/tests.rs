use super::*;
use install::*;
use std::path::{Path, PathBuf};
fn temp() -> tempfile::TempDir {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/maintenance-tests");
    std::fs::create_dir_all(&p).unwrap();
    tempfile::tempdir_in(p).unwrap()
}
fn package(t: &Path, version: &str) -> PathBuf {
    let stage = t.join(format!("stage-{version}"));
    std::fs::create_dir(&stage).unwrap();
    for name in [
        "app.exe",
        "Kavychka.exe",
        "next/worker.py",
        "backend/kavychka_local/engine.py",
        "assets/components.json",
        "assets/character.json",
        "assets/voice/profile.json",
    ] {
        let p = stage.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, format!("fixture-{version}")).unwrap();
    }
    let stub = t.join(format!("stub-{version}"));
    std::fs::write(&stub, b"MZ fixture stub").unwrap();
    let out = t.join(format!("Setup-{version}.exe"));
    package::append(
        &stub,
        &stage,
        package::collect(&stage, version.into()).unwrap(),
        &out,
    )
    .unwrap();
    out
}
fn probe(_: &Path, c: &Current) -> Result<()> {
    let mut s = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
    let mut p = s.load()?;
    p.user_name = format!("settings {}", c.version);
    s.save(&p)?;
    Ok(())
}
#[cfg(windows)]
#[test]
#[ignore = "explicit native fixture; set KAVYCHKA_JUNCTION_TEST_ROOT to a temporary test directory"]
fn owned_cleanup_rejects_a_junction_to_another_owned_directory() {
    let scratch = PathBuf::from(
        std::env::var_os("KAVYCHKA_JUNCTION_TEST_ROOT")
            .expect("set an isolated junction fixture root"),
    );
    assert!(scratch.is_absolute());
    std::fs::create_dir_all(&scratch).unwrap();
    let t = tempfile::tempdir_in(scratch).unwrap();
    let versions = t.path().join("versions");
    let active = versions.join("1.0.0");
    std::fs::create_dir_all(active.join("nested")).unwrap();
    std::fs::write(active.join("nested/keep"), b"active version").unwrap();
    let stage = versions.join("stage-1.1.0");
    let script = t.path().join("create-junction.ps1");
    std::fs::write(&script, b"param([string]$Link,[string]$Target)\r\nNew-Item -ItemType Junction -Path $Link -Target $Target -ErrorAction Stop | Out-Null\r\n").unwrap();
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = std::process::Command::new(powershell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Link")
        .arg(stage.to_string_lossy().trim_start_matches("\\\\?\\"))
        .arg("-Target")
        .arg(active.to_string_lossy().trim_start_matches("\\\\?\\"));
    crate::windows::hide(&mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "junction fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(delete_owned_tree(&stage, &versions).is_err());
    assert!(delete_owned_tree(&stage.join("nested"), &versions).is_err());
    assert_eq!(
        std::fs::read(active.join("nested/keep")).unwrap(),
        b"active version"
    );
    std::fs::remove_dir(stage).unwrap();
}
#[cfg(windows)]
#[test]
#[ignore = "explicit native fixture; set KAVYCHKA_SHORTCUT_TEST_ROOT to a temporary test directory"]
fn shortcut_staging_preserves_existing_named_neighbors() {
    let scratch = PathBuf::from(
        std::env::var_os("KAVYCHKA_SHORTCUT_TEST_ROOT")
            .expect("set an isolated shortcut fixture root"),
    );
    assert!(scratch.is_absolute());
    std::fs::create_dir_all(&scratch).unwrap();
    let t = tempfile::tempdir_in(scratch).unwrap();
    crate::windows::verify_shortcuts(&t.path().join("evidence")).unwrap();
}
#[test]
fn update_execution_rechecks_digest_manifest_and_denies_replacement() {
    let t = temp();
    let setup = package(t.path(), "0.1.0");
    let descriptor = updates::Release {
        version: "0.1.0".into(),
        url: String::new(),
        sha256: digest(&setup).unwrap(),
        bytes: setup.metadata().unwrap().len(),
        notes: String::new(),
    };
    let guard = updates::executable_guard(&descriptor, &setup).unwrap();
    #[cfg(windows)]
    assert!(
        std::fs::OpenOptions::new()
            .write(true)
            .open(&setup)
            .is_err()
    );
    drop(guard);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&setup)
        .unwrap()
        .write_all(b"changed after verification")
        .unwrap();
    assert!(updates::executable_guard(&descriptor, &setup).is_err());
}
#[test]
fn portable_folder_move_update_and_rollback_keep_owned_paths_and_external_models() {
    let t = temp();
    let v1 = package(t.path(), "0.1.0");
    let v2 = package(t.path(), "0.2.0");
    let bundle = t.path().join("portable");
    let moved = t.path().join("portable moved");
    let launcher = t.path().join("launcher.exe");
    std::fs::write(&launcher, b"MZ test launcher").unwrap();
    portable::create(&v1, &launcher, &bundle).unwrap();
    let root = portable::prepare(&bundle, probe).unwrap();
    let old = current(&root).unwrap().unwrap();
    let external = t.path().join("external");
    std::fs::create_dir(&external).unwrap();
    {
        let mut s = kavychka_storage::sandbox::Sandbox::open(&old.data).unwrap();
        let mut p = s.load().unwrap();
        p.generation.model_paths.insert(
            "text-gemma4-12b".into(),
            old.common
                .join("models/text-gemma4-12b")
                .display()
                .to_string(),
        );
        p.generation
            .model_paths
            .insert("text-1.7b".into(), external.display().to_string());
        s.save(&p).unwrap();
    }
    apply(&root, &old.common, &v2, |_, _| Ok(())).unwrap();
    std::fs::rename(&bundle, &moved).unwrap();
    let root = portable::prepare(&moved, |_, _| Ok(())).unwrap();
    let c = current(&root).unwrap().unwrap();
    assert_eq!(c.version, "0.2.0");
    assert_eq!(c.common, moved.canonicalize().unwrap().join("data"));
    {
        let s = kavychka_storage::sandbox::Sandbox::open(&c.data).unwrap();
        let p = s.load().unwrap();
        assert_eq!(
            kavychka_storage::transfer::owned_tail(
                Path::new(&p.generation.model_paths["text-gemma4-12b"]),
                &c.common
            )
            .unwrap(),
            Path::new("models/text-gemma4-12b")
        );
        assert_eq!(
            p.generation.model_paths["text-1.7b"],
            external.display().to_string()
        );
    }
    rollback(&root).unwrap();
    let c = current(&root).unwrap().unwrap();
    assert_eq!(c.version, "0.1.0");
    let s = kavychka_storage::sandbox::Sandbox::open(&c.data).unwrap();
    assert!(
        kavychka_storage::transfer::owned_tail(
            Path::new(&s.load().unwrap().generation.model_paths["text-gemma4-12b"]),
            &c.common
        )
        .is_some()
    );
    assert!(!bundle.exists());
    assert!(!t.path().join("Kavychka-next-install.json").exists());
}

#[cfg(windows)]
#[test]
fn update_accepts_the_same_storage_with_either_windows_prefix_and_rejects_other_storage() {
    for canonical_first in [true, false] {
        let t = temp();
        let root = t.path().join("program");
        let ordinary = t.path().join("data");
        std::fs::create_dir(&ordinary).unwrap();
        let canonical = ordinary.canonicalize().unwrap();
        assert_ne!(ordinary, canonical);
        let (first, next) = if canonical_first {
            (&canonical, &ordinary)
        } else {
            (&ordinary, &canonical)
        };
        let v1 = package(t.path(), "0.1.0");
        let v2 = package(t.path(), "0.2.0");
        let v3 = package(t.path(), "0.3.0");
        apply(&root, first, &v1, probe).unwrap();
        let old = current(&root).unwrap().unwrap();
        let before = {
            let s = kavychka_storage::sandbox::Sandbox::open(&old.data).unwrap();
            serde_json::to_value(s.load().unwrap()).unwrap()
        };
        apply(&root, next, &v2, |_, _| Ok(())).unwrap();
        let updated = current(&root).unwrap().unwrap();
        assert_eq!(updated.version, "0.2.0");
        assert_eq!(updated.common, old.common);
        assert_eq!(updated.data, old.data);
        assert_eq!(updated.common.canonicalize().unwrap(), canonical);
        {
            let s = kavychka_storage::sandbox::Sandbox::open(&updated.data).unwrap();
            assert_eq!(serde_json::to_value(s.load().unwrap()).unwrap(), before);
        }
        let other = t.path().join("data-other");
        let error = apply(&root, &other, &v3, |_, _| Ok(())).unwrap_err();
        assert!(error.to_string().contains("existing data folder"));
        assert!(!other.exists());
        assert_eq!(current(&root).unwrap().unwrap().version, "0.2.0");
        rollback(&root).unwrap();
        let restored = current(&root).unwrap().unwrap();
        assert_eq!(restored.version, "0.1.0");
        assert_eq!(restored.common.canonicalize().unwrap(), canonical);
        let s = kavychka_storage::sandbox::Sandbox::open(&restored.data).unwrap();
        assert_eq!(serde_json::to_value(s.load().unwrap()).unwrap(), before);
    }
}
#[test]
fn reviewed_removal_preserves_data_external_models_and_reinstall_then_full_is_explicit() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("common");
    let local = t.path().join("private-local");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir(&common).unwrap();
    std::fs::write(
        common.join("storage-owner.json"),
        br#"{"application":"Kavychka","version":1}"#,
    )
    .unwrap();
    let external = t.path().join("external-model");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(external.join("weights"), b"never remove external").unwrap();
    let v1 = package(t.path(), "0.1.0");
    let c = apply(&root, &common, &v1, probe).unwrap();
    register_locator(&local, &root, &c).unwrap();
    {
        let mut s = kavychka_storage::sandbox::Sandbox::open(&c.data).unwrap();
        let mut p = s.load().unwrap();
        p.generation
            .model_paths
            .insert("text-gemma4-12b".into(), external.display().to_string());
        s.save(&p).unwrap();
    }
    let before = digest(&c.data.join("shell.sqlite3")).unwrap();
    let plan = removal::preview(&root, false).unwrap();
    assert_eq!(plan.external_paths_retained, vec![external.clone()]);
    {
        let _busy = kavychka_storage::sandbox::Sandbox::open(&c.data).unwrap();
        assert!(removal::apply(&plan, &local, |_| Ok(())).is_err());
        assert!(root.join("current.json").exists());
    }
    removal::apply(&plan, &local, |_| Ok(())).unwrap();
    assert!(c.data.join("shell.sqlite3").exists());
    assert_eq!(before, digest(&c.data.join("shell.sqlite3")).unwrap());
    assert!(locate(&local).unwrap().is_some());
    assert!(!root.join("versions").exists());
    let c = apply(&root, &common, &v1, |_, c| {
        let s = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
        assert_eq!(s.load()?.user_name, "settings 0.1.0");
        Ok(())
    })
    .unwrap();
    register_locator(&local, &root, &c).unwrap();
    let stale = removal::preview(&root, true).unwrap();
    std::fs::write(common.join("new-file"), b"foreign root file").unwrap();
    assert!(removal::preview(&root, true).is_err());
    std::fs::remove_file(common.join("new-file")).unwrap();
    std::fs::create_dir_all(common.join("cache")).unwrap();
    std::fs::write(common.join("cache/new-file"), b"added since preview").unwrap();
    assert!(removal::apply(&stale, &local, |_| Ok(())).is_err());
    let full = removal::preview(&root, true).unwrap();
    removal::apply(&full, &local, |_| Ok(())).unwrap();
    assert!(!common.exists());
    assert!(locate(&local).unwrap().is_none());
    assert_eq!(
        std::fs::read(external.join("weights")).unwrap(),
        b"never remove external"
    );
}
#[test]
fn offline_removal_retains_data_and_refuses_full_or_pending_transaction() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("common");
    let local = t.path().join("local");
    std::fs::create_dir(&local).unwrap();
    let v = package(t.path(), "0.1.0");
    let c = apply(&root, &common, &v, probe).unwrap();
    register_locator(&local, &root, &c).unwrap();
    let hash = digest(&c.data.join("shell.sqlite3")).unwrap();
    let detached = t.path().join("detached");
    // Both resolved paths are children of this temporary fixture, never a real disk.
    assert!(common.starts_with(t.path()) && detached.starts_with(t.path()));
    std::fs::rename(&common, &detached).unwrap();
    let p = removal::preview(&root, false).unwrap();
    assert!(!p.data_available);
    assert!(removal::preview(&root, true).is_err());
    let journal = root.join("transaction.json");
    let old = std::fs::read(&journal).unwrap();
    crate::atomic(
        &journal,
        &serde_json::json!({"format":1,"state":"switching"}),
    )
    .unwrap();
    assert!(removal::apply(&p, &local, |_| Ok(())).is_err());
    std::fs::write(&journal, old).unwrap();
    removal::apply(&p, &local, |_| Ok(())).unwrap();
    assert!(!common.exists());
    assert!(local.join("Kavychka-next-install.json").exists());
    std::fs::rename(&detached, &common).unwrap();
    assert_eq!(hash, digest(&c.data.join("shell.sqlite3")).unwrap());
    apply(&root, &common, &v, probe).unwrap();
}
#[test]
fn update_preserves_data_weights_and_can_rollback_without_losing_new_work() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("Общая папка");
    std::fs::create_dir(&common).unwrap();
    std::fs::create_dir(common.join("models")).unwrap();
    std::fs::write(common.join("models/weights"), b"never duplicate").unwrap();
    let v1 = package(t.path(), "0.1.0");
    let v2 = package(t.path(), "0.2.0");
    apply(&root, &common, &v1, probe).unwrap();
    apply(&root, &common, &v2, probe).unwrap();
    assert_eq!(current(&root).unwrap().unwrap().version, "0.2.0");
    {
        let mut s =
            kavychka_storage::sandbox::Sandbox::open(&common.join(".kavychka-next")).unwrap();
        let mut p = s.load().unwrap();
        p.user_name = "new work after update".into();
        s.save(&p).unwrap();
    }
    rollback(&root).unwrap();
    let c = current(&root).unwrap().unwrap();
    assert_eq!(c.version, "0.1.0");
    let s = kavychka_storage::sandbox::Sandbox::open(&c.data).unwrap();
    assert_eq!(s.load().unwrap().user_name, "settings 0.1.0");
    drop(s);
    let backups: Vec<_> = std::fs::read_dir(root.join("recovery"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(
        backups
            .iter()
            .all(|p| !p.join("models").exists() && !p.join("components").exists())
    );
    assert_eq!(
        std::fs::read(common.join("models/weights")).unwrap(),
        b"never duplicate"
    );
    let after = backups
        .iter()
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("after-failed-")
        })
        .unwrap();
    let s = kavychka_storage::sandbox::Sandbox::open(after).unwrap();
    assert_eq!(s.load().unwrap().user_name, "new work after update");
}
#[test]
fn failed_probe_restores_previous_preferences_and_recover_is_idempotent() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    apply(&root, &common, &v1, probe).unwrap();
    assert!(
        apply(&root, &common, &v2, |p, c| {
            probe(p, c)?;
            Err("simulated startup failure".into())
        })
        .is_err()
    );
    assert_eq!(current(&root).unwrap().unwrap().version, "1.0.0");
    recover(&root).unwrap();
    let s = kavychka_storage::sandbox::Sandbox::open(&common.join(".kavychka-next")).unwrap();
    assert_eq!(s.load().unwrap().user_name, "settings 1.0.0");
}
#[test]
fn corrupt_package_unknown_folder_and_locked_data_never_activate() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    apply(&root, &common, &v1, probe).unwrap();
    let store = kavychka_storage::sandbox::Sandbox::open(&common.join(".kavychka-next")).unwrap();
    assert!(apply(&root, &common, &v2, probe).is_err());
    assert_eq!(current(&root).unwrap().unwrap().version, "1.0.0");
    drop(store);
    use std::io::{Seek, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(&v2).unwrap();
    f.seek(std::io::SeekFrom::End(-90)).unwrap();
    f.write_all(b"XX").unwrap();
    drop(f);
    assert!(apply(&root, &common, &v2, probe).is_err());
    assert_eq!(current(&root).unwrap().unwrap().version, "1.0.0");
    let unknown = t.path().join("unknown");
    std::fs::create_dir(&unknown).unwrap();
    std::fs::write(unknown.join("keep"), b"keep").unwrap();
    assert!(owned(&unknown).is_err());
    assert_eq!(std::fs::read(unknown.join("keep")).unwrap(), b"keep");
}
#[test]
fn interrupted_activation_restores_pointer_and_quarantines_candidate_changes() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    apply(&root, &common, &v1, probe).unwrap();
    apply(&root, &common, &v2, probe).unwrap();
    let mut j: Journal =
        serde_json::from_slice(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    j.state = "activating".into();
    atomic(&root.join("transaction.json"), &j).unwrap();
    recover(&root).unwrap();
    assert_eq!(current(&root).unwrap().unwrap().version, "1.0.0");
    let first = std::fs::read(root.join("current.json")).unwrap();
    recover(&root).unwrap();
    assert_eq!(first, std::fs::read(root.join("current.json")).unwrap());
}
#[test]
fn manifest_paths_and_versions_cannot_escape_or_downgrade() {
    let t = temp();
    for path in [
        "../escape",
        "C:/escape",
        "/escape",
        "assets//file",
        "file.",
        "folder\\file",
    ] {
        assert!(clean_path(t.path(), path).is_err())
    }
    for version in ["1.2", "1.2.3.4", "01.2.3", "1.2.3-beta", "../1"] {
        assert!(package::version(version).is_err())
    }
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    apply(&root, &common, &v1, probe).unwrap();
    assert!(apply(&root, &common, &v1, probe).is_err());
}

#[test]
fn corrupt_candidate_and_incomplete_rescue_do_not_block_safe_rollback() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    apply(&root, &common, &v1, probe).unwrap();
    apply(&root, &common, &v2, probe).unwrap();
    let mut j: Journal =
        serde_json::from_slice(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    j.state = "activating".into();
    atomic(&root.join("transaction.json"), &j).unwrap();
    let preserved = root
        .join("recovery")
        .join(format!("after-failed-{}", j.created.timestamp_millis()));
    std::fs::create_dir(&preserved).unwrap();
    std::fs::write(preserved.join("incomplete"), b"partial").unwrap();
    let broken = b"not a sqlite database, retain these exact bytes";
    std::fs::write(j.candidate.data.join("shell.sqlite3"), broken).unwrap();
    recover(&root).unwrap();
    assert_eq!(
        std::fs::read(preserved.join("shell.sqlite3")).unwrap(),
        broken
    );
    let s = kavychka_storage::sandbox::Sandbox::open(&j.candidate.data).unwrap();
    assert_eq!(s.load().unwrap().user_name, "settings 1.0.0");
    drop(s);
    recover(&root).unwrap();
    assert_eq!(current(&root).unwrap().unwrap().version, "1.0.0");
}
#[test]
fn damaged_recovery_copy_is_rejected_before_current_data_is_touched() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    apply(&root, &common, &v1, probe).unwrap();
    apply(&root, &common, &v2, probe).unwrap();
    let mut j: Journal =
        serde_json::from_slice(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    let db = j.candidate.data.join("shell.sqlite3");
    let before = digest(&db).unwrap();
    std::fs::write(
        j.backup.as_ref().unwrap().join("shell.sqlite3"),
        b"corrupted backup",
    )
    .unwrap();
    j.state = "activating".into();
    atomic(&root.join("transaction.json"), &j).unwrap();
    assert!(recover(&root).is_err());
    assert_eq!(digest(&db).unwrap(), before);
    assert_eq!(current(&root).unwrap().unwrap().version, "1.1.0");
}
#[test]
fn chosen_budget_locator_and_first_migration_rollback_keep_source() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("shared");
    let source = t.path().join("old");
    let local = t.path().join("local");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&local).unwrap();
    std::fs::write(source.join("settings.json"), b"original").unwrap();
    request_recovery_limit(&root, &common, 3_000_000_000).unwrap();
    let v = package(t.path(), "1.0.0");
    let c = apply_with_source(&root, &common, &v, Some(&source), |p, c| {
        probe(p, c)?;
        let mut s = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
        s.apply_pending_recovery_limit()?;
        assert_eq!(s.load()?.desktop.recovery_limit_bytes, 3_000_000_000);
        Ok(())
    })
    .unwrap();
    register_locator(&local, &root, &c).unwrap();
    let l = locate(&local).unwrap().unwrap();
    assert_eq!(l.legacy, Some(source.clone()));
    assert_eq!(l.common_root, common);
    assert!(rollback_available(&root));
    assert!(rollback(&root).unwrap().is_none());
    assert_eq!(
        std::fs::read(source.join("settings.json")).unwrap(),
        b"original"
    );
    assert!(c.data.join("shell.sqlite3").is_file());
    assert!(apply(&t.path().join("nested"), t.path(), &v, probe).is_err());
}
fn retention_fixture(t: &Path) -> (PathBuf, PathBuf, chrono::DateTime<chrono::Utc>) {
    let root = t.join("program");
    drop(owned(&root).unwrap());
    let common = t.join("common");
    let data = common.join(".kavychka-next");
    let mut store = kavychka_storage::sandbox::Sandbox::open(&data).unwrap();
    let mut prefs = store.load().unwrap();
    prefs.user_name = "Retention fixture".into();
    store.save(&prefs).unwrap();
    drop(store);
    let now = "2026-10-23T12:00:00Z".parse().unwrap();
    (root, common, now)
}
fn retention_journal(
    root: &Path,
    common: &Path,
    backup: &Path,
    now: chrono::DateTime<chrono::Utc>,
    state: &str,
) {
    atomic(
        &root.join("transaction.json"),
        &Journal {
            format: 1,
            state: state.into(),
            candidate: Current {
                format: 1,
                version: "5.0.0".into(),
                common: common.into(),
                data: common.join(".kavychka-next"),
                legacy: None,
            },
            previous: None,
            backup: Some(backup.into()),
            created: now,
            recovery_limit: 2_000_000_000,
        },
    )
    .unwrap();
}
fn indexed_backup(
    root: &Path,
    common: &Path,
    name: &str,
    version: &str,
    when: chrono::DateTime<chrono::Utc>,
) -> PathBuf {
    let path = root.join("recovery").join(name);
    snapshot(&common.join(".kavychka-next"), &path).unwrap();
    register_backup(root, &path, version, when).unwrap();
    path
}
#[test]
fn retention_uses_explicit_version_and_sequence_not_directory_sort_or_modified_time() {
    let t = temp();
    let (root, common, now) = retention_fixture(t.path());
    let oldest = indexed_backup(
        &root,
        &common,
        "before-zzz",
        "1.0.0",
        now - chrono::Duration::hours(3),
    );
    let middle = indexed_backup(
        &root,
        &common,
        "before-aaa",
        "2.0.0",
        now - chrono::Duration::hours(2),
    );
    let latest = indexed_backup(
        &root,
        &common,
        "before-mmm",
        "3.0.0",
        now - chrono::Duration::hours(1),
    );
    retention_journal(&root, &common, &latest, now, "complete");
    prune_ready_at(&root, 2_000_000_000, now).unwrap();
    assert!(!oldest.exists());
    assert!(middle.exists() && latest.exists());
    let duplicate = indexed_backup(&root, &common, "before-bbb", "2.0.0", now);
    prune_ready_at(&root, 2_000_000_000, now).unwrap();
    assert!(!middle.exists());
    assert!(duplicate.exists() && latest.exists());
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("recovery-index.json")).unwrap()).unwrap();
    assert_eq!(index["backups"].as_array().unwrap().len(), 2);
}
#[test]
fn expired_backups_are_removed_only_after_verified_transition_and_last_usable_is_protected() {
    let t = temp();
    let (root, common, now) = retention_fixture(t.path());
    let first = indexed_backup(
        &root,
        &common,
        "before-first",
        "1.0.0",
        now - chrono::Duration::days(10),
    );
    let last = indexed_backup(
        &root,
        &common,
        "before-last",
        "2.0.0",
        now - chrono::Duration::days(9),
    );
    retention_journal(&root, &common, &last, now, "probing");
    prune_ready_at(&root, 2_000_000_000, now).unwrap();
    assert!(first.exists() && last.exists());
    retention_journal(&root, &common, &last, now, "complete");
    prune_ready_at(&root, 2_000_000_000, now).unwrap();
    assert!(!first.exists());
    assert!(last.exists());
    assert!(prune_ready_at(&root, 1, now).is_err());
    assert!(last.exists());
}
#[test]
fn unknown_or_corrupt_recovery_copies_are_retained_and_do_not_displace_the_last_valid_backup() {
    let t = temp();
    let (root, common, now) = retention_fixture(t.path());
    let good = indexed_backup(
        &root,
        &common,
        "before-good",
        "1.0.0",
        now - chrono::Duration::days(10),
    );
    let broken = indexed_backup(
        &root,
        &common,
        "before-broken",
        "2.0.0",
        now - chrono::Duration::days(9),
    );
    std::fs::write(broken.join("shell.sqlite3"), b"corrupt fixture").unwrap();
    let unknown = root.join("recovery/before-unindexed");
    std::fs::create_dir(&unknown).unwrap();
    std::fs::write(unknown.join("keep"), b"unknown copy").unwrap();
    retention_journal(&root, &common, &broken, now, "complete");
    prune_ready_at(&root, 2_000_000_000, now).unwrap();
    assert!(good.exists() && broken.exists() && unknown.exists());
}
#[test]
fn update_snapshot_does_not_nest_profile_archives_recovery_models_or_logs_and_rollback_keeps_them()
{
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("common");
    let v1 = package(t.path(), "1.0.0");
    let v2 = package(t.path(), "1.1.0");
    let first = apply(&root, &common, &v1, probe).unwrap();
    let protected = [
        "exports/profile.zip",
        "recovery/profiles/fixture/before.zip",
        "models/model.gguf",
        "runtime/runtime.bin",
        "logs/private.log",
    ];
    for path in protected {
        let file = first.data.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, path.as_bytes()).unwrap();
    }
    assert!(
        apply(&root, &common, &v2, |p, c| {
            probe(p, c)?;
            Err("fixture failed probe".into())
        })
        .is_err()
    );
    for path in protected {
        assert_eq!(
            std::fs::read(first.data.join(path)).unwrap(),
            path.as_bytes()
        );
    }
    let j: Journal =
        serde_json::from_slice(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    let backup = j.backup.unwrap();
    for name in ["exports", "recovery", "models", "runtime", "logs"] {
        assert!(!backup.join(name).exists());
    }
}

#[test]
fn moving_storage_keeps_update_rollback_and_rebases_restored_paths_without_changing_backup() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("common");
    let local = t.path().join("local");
    let target = t.path().canonicalize().unwrap().join("moved");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(common.join("models")).unwrap();
    std::fs::write(common.join("models/owned.gguf"), b"model fixture").unwrap();
    let v1 = package(t.path(), "0.1.0");
    let v2 = package(t.path(), "0.2.0");
    let first = apply(&root, &common, &v1, probe).unwrap();
    {
        let mut s = kavychka_storage::sandbox::Sandbox::open(&first.data).unwrap();
        let mut p = s.load().unwrap();
        p.generation.model_paths.insert(
            "text-gemma4-12b".into(),
            common.join("models/owned.gguf").to_string_lossy().into(),
        );
        s.save(&p).unwrap();
    }
    let c = apply(&root, &common, &v2, probe).unwrap();
    register_locator(&local, &root, &c).unwrap();
    let j: Journal =
        serde_json::from_slice(&std::fs::read(root.join("transaction.json")).unwrap()).unwrap();
    let backup = j.backup.unwrap();
    let before = digest(&backup.join("shell.sqlite3")).unwrap();
    let manifest_before = digest(&backup.join("backup.manifest.json")).unwrap();
    crate::relocate::apply(
        &root,
        &local,
        &target,
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    rollback(&root).unwrap();
    let current = current(&root).unwrap().unwrap();
    assert_eq!(current.version, "0.1.0");
    assert_eq!(current.common, target);
    let s = kavychka_storage::sandbox::Sandbox::open(&current.data).unwrap();
    assert_eq!(
        s.load().unwrap().generation.model_paths["text-gemma4-12b"],
        target.join("models/owned.gguf").to_string_lossy()
    );
    assert_eq!(before, digest(&backup.join("shell.sqlite3")).unwrap());
    assert_eq!(
        manifest_before,
        digest(&backup.join("backup.manifest.json")).unwrap()
    );
}

#[test]
fn colocated_legacy_library_moves_then_accepts_the_next_update_probe_without_old_path_dependency() {
    let t = temp();
    let root = t.path().join("program");
    let common = t.path().join("common");
    let local = t.path().join("local");
    let target = t.path().canonicalize().unwrap().join("moved");
    std::fs::create_dir_all(common.join("audio")).unwrap();
    std::fs::create_dir_all(common.join("models")).unwrap();
    std::fs::create_dir(&local).unwrap();
    std::fs::write(common.join("models/owned.gguf"), b"owned model fixture").unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/voice/default-en.wav"),
        common.join("audio/ready.wav"),
    )
    .unwrap();
    let db = rusqlite::Connection::open(common.join("library.sqlite3")).unwrap();
    db.execute_batch(include_str!("../../storage/src/legacy_fixture.sql"))
        .unwrap();
    db.execute(
        "UPDATE phrases SET audio_path='ready.wav' WHERE audio_path IS NOT NULL",
        [],
    )
    .unwrap();
    drop(db);
    std::fs::write(common.join("settings.json"),br#"{"SchemaVersion":2,"UserName":"Synthetic legacy owner","VoiceProvider":"System","TextGeneration":false,"AutomaticPreparation":false}"#).unwrap();
    let startup_probe = |_: &Path, c: &Current| -> Result<()> {
        let mut store = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
        let source = c.legacy.as_ref().ok_or("legacy hint missing")?;
        store.bootstrap_legacy(source, &std::sync::atomic::AtomicBool::new(false))?;
        if store.meta("legacy_semantics_v2")?.is_none() {
            let utc: chrono::DateTime<chrono::Utc> = "2026-10-24T09:00:00Z".parse()?;
            store.complete_legacy_semantics(
                kavychka_core::engine::Clock {
                    utc,
                    mono: 0,
                    date: utc.date_naive(),
                },
                |day, minute| {
                    Ok(day
                        .and_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)
                        .unwrap()
                        .and_utc())
                },
            )?;
            let mut p = store.load()?;
            p.desktop.setup_complete = true;
            p.generation.model_paths.insert(
                "text-gemma4-12b".into(),
                c.common.join("models/owned.gguf").to_string_lossy().into(),
            );
            store.save(&p)?;
        }
        let origin = store.origin()?.ok_or("origin missing")?;
        assert_eq!(origin.source.canonicalize()?, c.common.canonicalize()?);
        assert!(origin.snapshot.join("library.snapshot.sqlite3").is_file());
        assert_eq!(
            store.phrases()?.iter().filter(|r| r.id == "used").count(),
            1
        );
        assert_eq!(
            Path::new(&store.load()?.generation.model_paths["text-gemma4-12b"]),
            c.common.join("models/owned.gguf")
        );
        Ok(())
    };
    let v1 = package(t.path(), "0.1.0");
    let v2 = package(t.path(), "0.2.0");
    let first = apply_with_source(&root, &common, &v1, Some(&common), startup_probe).unwrap();
    register_locator(&local, &root, &first).unwrap();
    let before = digest(&common.join("library.sqlite3")).unwrap();
    let owned_before = digest(&first.data.join("shell.sqlite3")).unwrap();
    let moved = crate::relocate::apply(
        &root,
        &local,
        &target,
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(moved.legacy, Some(target.clone()));
    assert_eq!(before, digest(&common.join("library.sqlite3")).unwrap());
    assert_eq!(
        owned_before,
        digest(&first.data.join("shell.sqlite3")).unwrap()
    );
    startup_probe(Path::new("unused"), &moved).unwrap();
    let updated = apply(&root, &target, &v2, startup_probe).unwrap();
    assert_eq!(updated.legacy, Some(target.clone()));
    assert_eq!(updated.version, "0.2.0");
    rollback(&root).unwrap();
    let rolled = current(&root).unwrap().unwrap();
    startup_probe(Path::new("unused"), &rolled).unwrap();
    assert_eq!(rolled.legacy, Some(target));
}
