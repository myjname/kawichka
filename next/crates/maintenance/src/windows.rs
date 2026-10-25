use crate::{Result, install::Current};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub fn show_error(detail: &str) {
    #[cfg(windows)]
    unsafe {
        let text: Vec<u16> = detail.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Кавычка / Kavychka".encode_utf16().chain(Some(0)).collect();
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            0x10,
        );
    }
    #[cfg(not(windows))]
    eprintln!("{detail}");
}
pub fn hide(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}
pub fn disk_space(path: &Path, required: u64) -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        let p: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0;
        if windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            p.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if free < required {
            return Err("not enough free disk space; current installation retained".into());
        }
    }
    #[cfg(not(windows))]
    let _ = (path, required);
    Ok(())
}
pub fn system_powershell() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
}
pub fn replace(from: &Path, to: &Path) -> Result<()> {
    kavychka_storage::preview::no_link_chain(from)?;
    kavychka_storage::preview::no_link_chain(to)?;
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        let a: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if MoveFileExW(
            a.as_ptr(),
            b.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(from, to)?;
    Ok(())
}
fn run(mut cmd: std::process::Command) -> Result<()> {
    hide(&mut cmd);
    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err("Windows shortcut operation failed".into())
            };
        }
        if start.elapsed() > std::time::Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Windows shortcut operation timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
pub fn shortcut(path: &Path, target: &Path, arguments: &str) -> Result<()> {
    kavychka_storage::preview::no_link_chain(path)?;
    let parent = path.parent().ok_or("shortcut parent missing")?;
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension("next.lnk");
    kavychka_storage::preview::no_link_chain(&temp)?;
    let mut cmd = std::process::Command::new(system_powershell()?);
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:KAVYCHKA_SHORTCUT); $s.TargetPath=$env:KAVYCHKA_TARGET; $s.WorkingDirectory=$env:KAVYCHKA_WORKING; $s.Arguments=$env:KAVYCHKA_ARGUMENTS; $s.Save()"])
        .env("KAVYCHKA_SHORTCUT", &temp).env("KAVYCHKA_TARGET", target).env("KAVYCHKA_WORKING", target.parent().ok_or("shortcut target parent missing")?).env("KAVYCHKA_ARGUMENTS", arguments);
    run(cmd)?;
    replace(&temp, path)
}
fn known_folder(startup: bool) -> Result<PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{System::Com::CoTaskMemFree, UI::Shell::*};
        let id = if startup {
            &FOLDERID_Startup
        } else {
            &FOLDERID_Programs
        };
        let mut p = std::ptr::null_mut();
        if SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p) < 0 {
            return Err("Windows user folder unavailable".into());
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        CoTaskMemFree(p.cast());
        Ok(PathBuf::from(text))
    }
    #[cfg(not(windows))]
    {
        let _ = startup;
        Err("Windows integration is unavailable".into())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortcutBackup {
    format: u32,
    existed: bool,
    source: PathBuf,
    backup_sha256: Option<String>,
    installed_sha256: Option<String>,
}
fn integration(
    root: &Path,
    folder: &Path,
    name: &str,
    tag: &str,
    enabled: bool,
    arguments: &str,
) -> Result<()> {
    kavychka_storage::preview::no_link_chain(folder)?;
    kavychka_storage::preview::no_link_chain(root)?;
    if std::fs::read(root.join("install.owner"))? != b"Kavychka.Next.v1\n" {
        return Err("unknown install owner".into());
    }
    let path = folder.join(name);
    kavychka_storage::preview::no_link_chain(&path)?;
    let base = root.join("integration");
    kavychka_storage::preview::no_link_chain(&base)?;
    std::fs::create_dir_all(&base)?;
    let metadata = base.join(format!("{tag}.json"));
    let backup = base.join(format!("{tag}-before-next.lnk"));
    kavychka_storage::preview::no_link_chain(&metadata)?;
    kavychka_storage::preview::no_link_chain(&backup)?;
    let mut record = if metadata.exists() {
        serde_json::from_slice::<ShortcutBackup>(&std::fs::read(&metadata)?)?
    } else {
        if backup.exists() {
            return Err("unfinished shortcut backup retained for recovery".into());
        }
        let existed = path.exists();
        if existed {
            if path.metadata()?.len() > 1_000_000 {
                return Err("shortcut exceeds 1 MB".into());
            }
            std::fs::copy(&path, &backup)?;
        }
        let record = ShortcutBackup {
            format: 1,
            existed,
            source: path.clone(),
            backup_sha256: if existed {
                Some(crate::digest(&backup)?)
            } else {
                None
            },
            installed_sha256: None,
        };
        crate::atomic(&metadata, &record)?;
        record
    };
    if record.format != 1 || record.source != path {
        return Err("shortcut backup belongs to another location".into());
    }
    if path.exists() {
        let sha = crate::digest(&path)?;
        if record.installed_sha256.as_deref() != Some(&sha)
            && record.backup_sha256.as_deref() != Some(&sha)
        {
            return Err("Shortcut changed outside Kavychka; user replacement retained".into());
        }
    }
    if enabled {
        shortcut(&path, &root.join("Kavychka.exe"), arguments)?;
        record.installed_sha256 = Some(crate::digest(&path)?);
    } else {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        record.installed_sha256 = None;
    }
    crate::atomic(&metadata, &record)
}
fn restore_integration(root: &Path, folder: &Path, name: &str, tag: &str) -> Result<()> {
    let base = root.join("integration");
    let metadata = base.join(format!("{tag}.json"));
    kavychka_storage::preview::no_link_chain(&metadata)?;
    if !metadata.exists() {
        return Ok(());
    }
    let record: ShortcutBackup = serde_json::from_slice(&std::fs::read(metadata)?)?;
    let path = folder.join(name);
    kavychka_storage::preview::no_link_chain(&path)?;
    if record.format != 1 || record.source != path {
        return Err("shortcut restore location changed".into());
    }
    if record.existed {
        if path.exists() {
            let digest = crate::digest(&path)?;
            if record.installed_sha256.as_deref() != Some(&digest)
                && record.backup_sha256.as_deref() != Some(&digest)
            {
                return Err("shortcut changed outside the app; retained for manual review".into());
            }
        }
        let backup = base.join(format!("{tag}-before-next.lnk"));
        kavychka_storage::preview::no_link_chain(&backup)?;
        if record.backup_sha256.as_deref() != Some(&crate::digest(&backup)?) {
            return Err("shortcut backup integrity check failed".into());
        }
        let temp = path.with_extension("restore.lnk");
        kavychka_storage::preview::no_link_chain(&temp)?;
        std::fs::copy(backup, &temp)?;
        replace(&temp, &path)?;
    } else if path.exists() {
        if record.installed_sha256.as_deref() != Some(&crate::digest(&path)?) {
            return Err("shortcut changed outside the app; retained for manual review".into());
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}
pub fn startup_in(root: &Path, folder: &Path, enabled: bool) -> Result<()> {
    integration(
        root,
        folder,
        "KavychkaBreaks.lnk",
        "startup",
        enabled,
        "--background",
    )
}
pub fn startup(root: &Path, enabled: bool) -> Result<()> {
    startup_in(root, &known_folder(true)?, enabled)
}
pub fn restore_startup_in(root: &Path, folder: &Path) -> Result<()> {
    restore_integration(root, folder, "KavychkaBreaks.lnk", "startup")
}
pub fn restore_legacy_startup(root: &Path) -> Result<()> {
    restore_startup_in(root, &known_folder(true)?)?;
    restore_integration(root, &known_folder(false)?, "Кавычка.lnk", "startmenu")
}
pub fn register(root: &Path, _current: &Current) -> Result<()> {
    integration(
        root,
        &known_folder(false)?,
        "Кавычка.lnk",
        "startmenu",
        true,
        "",
    )
}
/// Uninstall removes only the exact shortcuts written by this installation.
/// It does not enable an older app's startup entry or delete user replacements.
pub fn unregister(root: &Path) -> Result<()> {
    crate::registration::remove(root)?;
    for (tag, folder, name) in [
        ("startup", known_folder(true)?, "KavychkaBreaks.lnk"),
        ("startmenu", known_folder(false)?, "Кавычка.lnk"),
    ] {
        unregister_in(root, &folder, name, tag)?;
    }
    Ok(())
}
pub fn unregister_in(root: &Path, folder: &Path, name: &str, tag: &str) -> Result<()> {
    let metadata = root.join("integration").join(format!("{tag}.json"));
    kavychka_storage::preview::no_link_chain(&metadata)?;
    if !metadata.exists() {
        return Ok(());
    }
    if metadata.metadata()?.len() > 16000 {
        return Err("Shortcut record too large".into());
    }
    let record: ShortcutBackup = serde_json::from_slice(&std::fs::read(metadata)?)?;
    let target = folder.join(name);
    kavychka_storage::preview::no_link_chain(&target)?;
    if record.format != 1 || record.source != target {
        return Err("Shortcut removal location changed".into());
    }
    if target.exists() && record.installed_sha256.as_deref() == Some(&crate::digest(&target)?) {
        std::fs::remove_file(target)?;
    }
    Ok(())
}
/// Native integration QA uses only a new caller-provided fixture directory.
pub fn verify_shortcuts(evidence: &Path) -> Result<()> {
    if !evidence.is_absolute() || evidence.exists() {
        return Err("new absolute shortcut evidence folder required".into());
    }
    std::fs::create_dir_all(evidence)?;
    let root = evidence.join("program");
    let folder = evidence.join("simulated-startup");
    std::fs::create_dir(&folder)?;
    let guard = crate::install::owned(&root)?;
    drop(guard);
    std::fs::write(root.join("Kavychka.exe"), b"fixture; never executed")?;
    let path = folder.join("KavychkaBreaks.lnk");
    let old = evidence.join("old.exe");
    std::fs::write(&old, b"old; never executed")?;
    shortcut(&path, &old, "")?;
    let old_hash = crate::digest(&path)?;
    startup_in(&root, &folder, true)?;
    if crate::digest(&path)? == old_hash {
        return Err("startup target not replaced".into());
    }
    startup_in(&root, &folder, false)?;
    if path.exists() {
        return Err("startup disable failed".into());
    }
    restore_startup_in(&root, &folder)?;
    if crate::digest(&path)? != old_hash {
        return Err("original shortcut not restored".into());
    }
    let clean = evidence.join("fresh-program");
    let guard = crate::install::owned(&clean)?;
    drop(guard);
    std::fs::write(clean.join("Kavychka.exe"), b"fixture")?;
    let empty = evidence.join("fresh-startup");
    std::fs::create_dir(&empty)?;
    std::fs::write(empty.join("unrelated.txt"), b"keep")?;
    startup_in(&clean, &empty, true)?;
    restore_startup_in(&clean, &empty)?;
    if empty.join("KavychkaBreaks.lnk").exists()
        || std::fs::read(empty.join("unrelated.txt"))? != b"keep"
    {
        return Err("initial absence or unrelated file changed".into());
    }
    startup_in(&clean, &empty, true)?;
    std::fs::write(empty.join("KavychkaBreaks.lnk"), b"external edit")?;
    if restore_startup_in(&clean, &empty).is_ok() {
        return Err("external shortcut edits must be retained".into());
    }
    crate::atomic(
        &evidence.join("result.json"),
        &serde_json::json!({"passed":true,"real_windows_shortcuts":true,"replacement_disable_restore":true,"initial_absence_restored":true,"unrelated_files_preserved":true,"external_edits_retained":true,"system_startup_changed":false}),
    )
}
