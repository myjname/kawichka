use super::*;
fn temp() -> tempfile::TempDir {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/p5-move-tests");
    fs::create_dir_all(&path).unwrap();
    tempfile::tempdir_in(path.canonicalize().unwrap()).unwrap()
}
fn fixture(at: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = at.join("program");
    let local = at.join("local");
    let common = at.join("Общая папка");
    let target = at.join("Новая папка");
    fs::create_dir_all(&local).unwrap();
    drop(install::owned(&root).unwrap());
    let current = Current {
        format: 1,
        version: "0.1.0".into(),
        data: common.join(".kavychka-next"),
        common: common.clone(),
        legacy: None,
    };
    let mut s = Sandbox::open(&current.data).unwrap();
    fs::create_dir_all(common.join("models")).unwrap();
    fs::write(
        common.join("models/owned.gguf"),
        b"owned model weights, no download",
    )
    .unwrap();
    fs::create_dir_all(current.data.join("components/runtime")).unwrap();
    fs::write(
        current.data.join("components/runtime/python.exe"),
        b"runtime fixture",
    )
    .unwrap();
    fs::create_dir_all(current.data.join("exports")).unwrap();
    fs::write(current.data.join("exports/keep.zip"), b"user archive").unwrap();
    let mut p = s.load().unwrap();
    p.user_name = "Synthetic owner".into();
    p.desktop.setup_complete = true;
    p.generation.auto_daily = false;
    p.generation.model_paths.insert(
        "text-gemma4-12b".into(),
        common.join("models/owned.gguf").to_string_lossy().into(),
    );
    p.generation.model_paths.insert(
        "tts-cosyvoice3".into(),
        at.join("external/voice").to_string_lossy().into(),
    );
    s.save(&p).unwrap();
    s.put_meta(
        "fixture_ignored_metadata",
        &serde_json::json!({"preserve":true}),
    )
    .unwrap();
    drop(s);
    atomic(&root.join("current.json"), &current).unwrap();
    install::register_locator(&local, &root, &current).unwrap();
    (root, local, common, target)
}
#[test]
fn whole_common_move_rebases_owned_paths_reuses_models_keeps_source_and_reopens_old_launch_path() {
    let t = temp();
    let (root, local, old, target) = fixture(t.path());
    let cancel = AtomicBool::new(false);
    let e = estimate(&root, &target, &cancel).unwrap();
    assert!(e.bytes > 0 && e.files >= 5 && e.peak_bytes > e.bytes);
    assert_eq!(e.breakdown.values().sum::<u64>(), e.bytes);
    assert_eq!(
        e.breakdown["models"],
        b"owned model weights, no download".len() as u64
    );
    assert_eq!(e.breakdown["runtimes"], b"runtime fixture".len() as u64);
    let before = digest(&old.join(".kavychka-next/shell.sqlite3")).unwrap();
    let c = apply(&root, &local, &target, &cancel, |_| {}).unwrap();
    assert_eq!(c.common, target);
    assert_eq!(
        install::locate(&local).unwrap().unwrap().common_root,
        target
    );
    assert_eq!(
        digest(&old.join(".kavychka-next/shell.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        digest(&target.join("models/owned.gguf")).unwrap(),
        digest(&old.join("models/owned.gguf")).unwrap()
    );
    assert!(
        target
            .join(".kavychka-next/components/runtime/python.exe")
            .is_file()
    );
    assert!(target.join(".kavychka-next/exports/keep.zip").is_file());
    let s = Sandbox::open(&old.join(".kavychka-next")).unwrap();
    assert_eq!(s.root, c.data);
    let prefs = s.load().unwrap();
    assert_eq!(prefs.user_name, "Synthetic owner");
    assert_eq!(
        prefs.generation.model_paths["text-gemma4-12b"],
        target.join("models/owned.gguf").to_string_lossy()
    );
    assert_eq!(
        prefs.generation.model_paths["tts-cosyvoice3"],
        t.path().join("external/voice").to_string_lossy()
    );
    assert!(s.meta("fixture_ignored_metadata").unwrap().is_some());
    drop(s);
    // Completed relocation recovery must not resurrect the old app version after an update.
    let mut later = c;
    later.version = "0.2.0".into();
    atomic(&root.join("current.json"), &later).unwrap();
    recover(&root).unwrap();
    assert_eq!(install::current(&root).unwrap().unwrap().version, "0.2.0");
}
#[test]
fn rejects_running_writer_nonempty_nested_unavailable_and_cancel_before_switch() {
    let t = temp();
    let (root, local, old, target) = fixture(t.path());
    let cancel = AtomicBool::new(false);
    let writer = Sandbox::open(&old.join(".kavychka-next")).unwrap();
    assert!(apply(&root, &local, &target, &cancel, |_| {}).is_err());
    assert!(!target.exists());
    drop(writer);
    assert!(estimate(&root, &old.join("child"), &cancel).is_err());
    assert!(estimate(&root, &root.join("child"), &cancel).is_err());
    assert!(estimate(&root, &old.join("child/.."), &cancel).is_err());
    fs::create_dir(&target).unwrap();
    fs::write(target.join("user-file"), b"keep").unwrap();
    assert!(estimate(&root, &target, &cancel).is_err());
    let next = t.path().join("cancelled");
    let result = apply(&root, &local, &next, &cancel, |_| {
        cancel.store(true, Ordering::Release);
    });
    assert!(result.is_err());
    assert_eq!(install::current(&root).unwrap().unwrap().common, old);
    assert_eq!(fs::read(target.join("user-file")).unwrap(), b"keep");
    assert!(Sandbox::open(&old.join(".kavychka-next")).is_ok());
    if next.exists() {
        assert!(Sandbox::open(&next.join(".kavychka-next")).is_err());
    }
}
#[test]
fn injected_failures_before_commit_restore_pointers_after_commit_finish_without_source_deletion() {
    for stage in ["journal", "files", "probe", "pointer", "locator", "commit"] {
        let t = temp();
        let (root, local, old, target) = fixture(t.path());
        let r = apply_with(
            &root,
            &local,
            &target,
            &AtomicBool::new(false),
            |_| {},
            |at| {
                if at == stage {
                    Err("injected boundary failure".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(r.is_err(), "{stage}");
        recover(&root).unwrap();
        recover(&root).unwrap();
        let expected = if stage == "commit" { &target } else { &old };
        assert_eq!(
            &install::current(&root).unwrap().unwrap().common,
            expected,
            "{stage}"
        );
        assert_eq!(
            &install::locate(&local).unwrap().unwrap().common_root,
            expected,
            "{stage}"
        );
        assert!(old.join("models/owned.gguf").is_file());
        if stage != "commit" && target.exists() {
            assert!(
                Sandbox::open(&target.join(".kavychka-next")).is_err(),
                "{stage}"
            );
        }
    }
}
#[test]
fn real_child_process_exits_release_locks_and_recover_exactly_one_active_root() {
    for stage in ["journal", "files", "probe", "pointer", "locator", "commit"] {
        let t = temp();
        let (root, local, old, target) = fixture(t.path());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "relocate::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("KAVYCHKA_MOVE_TEST_ROOT", t.path())
            .env("KAVYCHKA_MOVE_TEST_STAGE", stage)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(77),
            "{stage}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        {
            let _guard = install::owned(&root).unwrap();
            recover(&root).unwrap();
        }
        let expected = if stage == "commit" { &target } else { &old };
        assert_eq!(
            &install::current(&root).unwrap().unwrap().common,
            expected,
            "{stage}"
        );
        assert_eq!(
            &install::locate(&local).unwrap().unwrap().common_root,
            expected,
            "{stage}"
        );
        assert!(Sandbox::open(&old.join(".kavychka-next")).is_ok());
        if stage != "commit" && target.exists() {
            assert!(Sandbox::open(&target.join(".kavychka-next")).is_err());
        }
    }
}
#[test]
#[ignore = "child process only, with a private synthetic fixture"]
fn crash_child() {
    let Some(at) = std::env::var_os("KAVYCHKA_MOVE_TEST_ROOT").map(PathBuf::from) else {
        return;
    };
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../work/p5-move-tests")
        .canonicalize()
        .unwrap();
    assert!(at.canonicalize().unwrap().starts_with(expected));
    let stage = std::env::var("KAVYCHKA_MOVE_TEST_STAGE").unwrap();
    if stage.starts_with("recover_") {
        let _guard = install::owned(&at.join("program")).unwrap();
        recover_with(&at.join("program"), |point| {
            if point == stage {
                std::process::exit(77);
            }
            Ok(())
        })
        .unwrap();
        panic!("recovery crash boundary not reached");
    }
    apply_with(
        &at.join("program"),
        &at.join("local"),
        &at.join("Новая папка"),
        &AtomicBool::new(false),
        |_| {},
        |point| {
            if point == stage {
                std::process::exit(77);
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("crash boundary not reached");
}

#[test]
fn crashes_during_pointer_recovery_can_repeat_without_losing_candidate_or_source() {
    for committed in [false, true] {
        for stage in ["recover_current", "recover_locator", "recover_fence"] {
            let t = temp();
            let (root, local, old, target) = fixture(t.path());
            let child = |point: &str| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "relocate::tests::crash_child",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("KAVYCHKA_MOVE_TEST_ROOT", t.path())
                    .env("KAVYCHKA_MOVE_TEST_STAGE", point)
                    .output()
                    .unwrap()
            };
            assert_eq!(
                child(if committed { "commit" } else { "pointer" })
                    .status
                    .code(),
                Some(77)
            );
            assert_eq!(child(stage).status.code(), Some(77));
            recover(&root).unwrap();
            recover(&root).unwrap();
            let expected = if committed { &target } else { &old };
            assert_eq!(&install::current(&root).unwrap().unwrap().common, expected);
            assert_eq!(
                &install::locate(&local).unwrap().unwrap().common_root,
                expected
            );
            assert!(
                old.join("models/owned.gguf").is_file()
                    && target.join("models/owned.gguf").is_file()
            );
            assert!(Sandbox::open(&old.join(".kavychka-next")).is_ok());
        }
    }
}

#[cfg(windows)]
#[test]
fn foreign_file_writer_is_rejected_before_switching() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = temp();
    let (root, local, old, target) = fixture(t.path());
    let writer = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(3)
        .open(old.join("models/owned.gguf"))
        .unwrap();
    assert!(apply(&root, &local, &target, &AtomicBool::new(false), |_| {}).is_err());
    drop(writer);
    assert_eq!(install::current(&root).unwrap().unwrap().common, old);
}
#[cfg(windows)]
#[test]
#[ignore = "explicit native fixture; sandbox disallows creating a junction"]
fn real_junction_is_rejected_without_traversing_its_target() {
    let t = temp();
    let (root, _local, old, _target) = fixture(t.path());
    let linked = t.path().join("other-owned-fixture");
    fs::create_dir(&linked).unwrap();
    fs::write(linked.join("keep"), b"do not traverse").unwrap();
    let junction = old.join("junction");
    let script = t.path().join("create-junction.ps1");
    fs::write(&script,b"param([string]$Link,[string]$Target)\r\nNew-Item -ItemType Junction -Path $Link -Target $Target -ErrorAction Stop | Out-Null\r\n").unwrap();
    let command = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = std::process::Command::new(command);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-Link")
        .arg(junction.to_string_lossy().trim_start_matches("\\\\?\\"))
        .arg("-Target")
        .arg(linked.to_string_lossy().trim_start_matches("\\\\?\\"));
    crate::windows::hide(&mut command);
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "owned junction fixture creation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        estimate(
            &root,
            &t.path().join("another-target"),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    fs::remove_dir(&junction).unwrap();
    assert_eq!(fs::read(linked.join("keep")).unwrap(), b"do not traverse");
}
