//! Opt-in real ENOSPC. Refuses ordinary disks; the companion script creates a
//! fresh <=160 MB virtual NTFS volume with a matching owner, GUID and image path.
use super::*;
use crate::{datafs, sandbox::Sandbox};
use std::{
    cell::RefCell,
    fs,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
};

struct Fill(PathBuf);
impl Drop for Fill {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn fill(root: &Path, name: &str) -> Fill {
    let path = root.join(name);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    let chunk = [91u8; 65536];
    let mut written = 0u64;
    loop {
        match file.write_all(&chunk) {
            Ok(()) => {
                written += chunk.len() as u64;
                assert!(
                    written < 160_000_000,
                    "refuse filling an unexpected larger volume"
                );
            }
            Err(error) => {
                assert!(
                    matches!(error.raw_os_error(), Some(112 | 39)),
                    "Expected actual Windows disk-full, got {error}"
                );
                break;
            }
        }
    }
    loop {
        match file.write_all(&chunk[..4096]) {
            Ok(()) => {}
            Err(error) => {
                assert!(matches!(error.raw_os_error(), Some(112 | 39)));
                break;
            }
        }
    }
    Fill(path)
}
#[test]
#[ignore = "requires explicit disposable virtual-volume fixture; ordinary disks refused"]
fn actual_virtual_volume_full_during_zip_and_restore_is_recoverable() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::*;
    let volume =
        PathBuf::from(std::env::var_os("KAVYCHKA_P5_FULL_VOLUME").expect("virtual volume missing"));
    let name = volume.to_string_lossy();
    assert!(name.len() == 3 && name.as_bytes()[1] == b':' && name.ends_with('\\'));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../work")
        .canonicalize()
        .unwrap();
    assert_ne!(
        name.chars().next().unwrap().to_ascii_lowercase(),
        workspace
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .chars()
            .next()
            .unwrap()
            .to_ascii_lowercase()
    );
    assert_ne!(
        name.chars().next().unwrap().to_ascii_lowercase(),
        std::env::var("SystemDrive")
            .unwrap()
            .chars()
            .next()
            .unwrap()
            .to_ascii_lowercase()
    );
    let marker: serde_json::Value =
        serde_json::from_slice(&fs::read(volume.join("kavychka-p5-volume.owner.json")).unwrap())
            .unwrap();
    assert_eq!(marker["kind"], "Kavychka.P5.ENOSPC.v1");
    assert_eq!(marker["format"], 1);
    let image = PathBuf::from(marker["image"].as_str().unwrap());
    assert!(image.canonicalize().unwrap().starts_with(&workspace));
    assert_eq!(image.extension().unwrap(), "vhdx");
    let result_file = PathBuf::from(marker["result"].as_str().unwrap());
    assert_eq!(result_file.file_name().unwrap(), "result.json");
    assert_eq!(
        result_file.parent().unwrap().canonicalize().unwrap(),
        image.parent().unwrap().canonicalize().unwrap()
    );
    assert!(!result_file.exists());
    let wide: Vec<u16> = volume.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0;
    let mut total = 0;
    assert_ne!(
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) },
        0
    );
    assert!((64_000_000..=160_000_000).contains(&total));
    let mut guid = [0u16; 64];
    assert_ne!(
        unsafe { GetVolumeNameForVolumeMountPointW(wide.as_ptr(), guid.as_mut_ptr(), 64) },
        0
    );
    assert_eq!(
        String::from_utf16_lossy(&guid[..guid.iter().position(|&v| v == 0).unwrap()])
            .to_lowercase(),
        marker["volume_guid"].as_str().unwrap().to_lowercase()
    );
    let mut label = [0u16; 64];
    assert_ne!(
        unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                label.as_mut_ptr(),
                64,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        },
        0
    );
    assert_eq!(
        String::from_utf16_lossy(&label[..label.iter().position(|&v| v == 0).unwrap()]),
        "Kavychka-P5-FULL"
    );
    let scratch = tempfile::tempdir_in(&workspace).unwrap();
    let (mut source, _) = super::tests::source(scratch.path());
    // Force the restore to need new database pages after the volume becomes full.
    for i in 0..16 {
        let mut row = super::tests::row(&format!("full-{i}"));
        row.text = format!("{i} {}", "Ж".repeat(3900));
        source.accept_phrase(&row, None).unwrap();
    }
    let mut noise = super::tests::wave(0);
    noise.truncate(44);
    let bytes = 2_000_000u32;
    noise[4..8].copy_from_slice(&(36 + bytes).to_le_bytes());
    noise[40..44].copy_from_slice(&bytes.to_le_bytes());
    let mut seed = 0x184a9321u32;
    for _ in 0..bytes / 2 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        noise.extend((seed as i16).to_le_bytes());
    }
    fs::write(source.root.join("audio/bulk.wav"), noise).unwrap();
    let before = super::tests::tree_hash(&source.root);
    let cancel = AtomicBool::new(false);
    let filler = RefCell::new(None);
    let output = volume.join("must-not-publish.zip");
    let result = CaptureSource::new(&source).unwrap().export(
        &output,
        &scratch.path().join("export-job"),
        ExportOptions::default(),
        "0.1.1",
        &cancel,
        |stage| {
            if stage == "archive-writing" {
                *filler.borrow_mut() = Some(fill(&volume, "zip-full.bin"));
            }
        },
    );
    assert!(
        filler.borrow().is_some(),
        "preflight must pass before inducing real ENOSPC"
    );
    assert!(
        matches!(result,Err(Error::Io(ref e)) if matches!(e.raw_os_error(),Some(112|39))),
        "{result:?}"
    );
    drop(filler);
    assert!(!output.exists());
    assert_eq!(before, super::tests::tree_hash(&source.root));
    let archive = super::tests::exported(&source, scratch.path(), ExportOptions::default());
    let mut target = Sandbox::open(&volume.join("restore-target")).unwrap();
    let inspected = inspect(
        &archive,
        &scratch.path().join("inspect"),
        2_000_000_000,
        &cancel,
        |_| {},
    )
    .unwrap();
    let mut plan = RestorePlan::prepare(&target, inspected, &cancel).unwrap();
    plan.prepare_files(
        CaptureSource::new(&target).unwrap(),
        "0.1.1",
        &cancel,
        |_| {},
    )
    .unwrap();
    let filler = Rc::new(RefCell::new(None));
    let f = filler.clone();
    let volume_for_hook = volume.clone();
    super::IO_BOUNDARY.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |point| {
            if point == "restore_file" && f.borrow().is_none() {
                *f.borrow_mut() = Some(fill(&volume_for_hook, "restore-full.bin"));
            }
        }))
    });
    struct ResetHook;
    impl Drop for ResetHook {
        fn drop(&mut self) {
            super::IO_BOUNDARY.with(|h| *h.borrow_mut() = None);
        }
    }
    let reset = ResetHook;
    let result = plan.apply(&mut target, &cancel);
    drop(reset);
    assert!(filler.borrow().is_some());
    assert!(
        matches!(result,Err(Error::Sqlite(rusqlite::Error::SqliteFailure(ref e,_))) if e.code==rusqlite::ErrorCode::DiskFull)
            || matches!(result,Err(Error::Io(ref e)) if matches!(e.raw_os_error(),Some(112|39))),
        "{result:?}"
    );
    filler.borrow_mut().take();
    assert!(target.phrases().unwrap().is_empty());
    assert!(!target.root.join("profile-restore.json").exists());
    assert!(
        fs::read_dir(target.root.join("audio"))
            .unwrap()
            .next()
            .is_none()
    );
    let backups: Vec<_> = fs::read_dir(target.root.join("recovery/profiles"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("before.zip"))
        .filter(|path| path.is_file())
        .collect();
    assert!(!backups.is_empty());
    for (i, backup) in backups.iter().enumerate() {
        inspect(
            backup,
            &scratch.path().join(format!("verify-retained-backup-{i}")),
            2_000_000_000,
            &cancel,
            |_| {},
        )
        .unwrap();
    }
    assert_eq!(before, super::tests::tree_hash(&source.root));
    datafs::json(&result_file,&serde_json::json!({"passed":true,"actual_virtual_filesystem_full":true,"ordinary_disks_refused":true,"zip_preflight_passed_then_windows_disk_full":true,"restore_file_published_then_actual_disk_full":true,"source_unchanged":true,"working_rows_rolled_back":true,"partial_archive_not_published":true,"current_data_and_backups_retained":true,"retained_backups_verified":true})).unwrap();
}
