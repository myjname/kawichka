//! Explicit reviewed removal. External model/preset roots are never destinations.
use crate::{Result, install};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format: u32,
    pub program: PathBuf,
    pub common: PathBuf,
    pub version: String,
    pub full: bool,
    #[serde(default = "yes")]
    pub data_available: bool,
    pub files: Vec<File>,
    pub bytes: u64,
    pub external_paths_retained: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub area: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
fn yes() -> bool {
    true
}
pub fn current(root: &Path) -> Result<install::Current> {
    let path = root.join("current.json");
    kavychka_storage::preview::no_link_chain(&path)?;
    if path.metadata()?.len() > 16000 {
        return Err("Installation pointer exceeds limit".into());
    }
    let c: install::Current = serde_json::from_slice(&fs::read(path)?)?;
    crate::package::version(&c.version)?;
    if c.format != 1 || !c.common.is_absolute() || c.data != c.common.join(".kavychka-next") {
        return Err("Invalid removal data pointer".into());
    }
    kavychka_storage::preview::no_link_chain(&c.common)?;
    Ok(c)
}
fn locator(local: &Path) -> Result<Option<install::Locator>> {
    let path = local.join("Kavychka-next-install.json");
    kavychka_storage::preview::no_link_chain(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    if path.metadata()?.len() > 16000 {
        return Err("Installation locator exceeds limit".into());
    }
    let l: install::Locator = serde_json::from_slice(&fs::read(path)?)?;
    if l.format != 1 || !l.common_root.is_absolute() || !l.program_root.is_absolute() {
        return Err("Invalid removal locator".into());
    }
    Ok(Some(l))
}
fn offline_ready(root: &Path) -> Result<()> {
    for name in ["transaction.json", "storage-move.json"] {
        let path = root.join(name);
        kavychka_storage::preview::no_link_chain(&path)?;
        if path.exists() {
            if path.metadata()?.len() > 2_000_000 {
                return Err("Maintenance journal exceeds limit".into());
            }
            let j: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
            if j["format"] != 1 || !matches!(j["state"].as_str(), Some("complete" | "rolled_back"))
            {
                return Err(
                    "Reconnect storage and finish the pending maintenance operation before removal"
                        .into(),
                );
            }
        }
    }
    Ok(())
}
fn scan(
    base: &Path,
    at: &Path,
    area: &str,
    out: &mut Vec<File>,
    cancel: &AtomicBool,
) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err("Removal preview cancelled".into());
    }
    if at.strip_prefix(base)?.components().count() > 64 {
        return Err("Removal folder nesting exceeds limit".into());
    }
    kavychka_storage::preview::no_link_chain(at)?;
    if !at.exists() {
        return Ok(());
    }
    if at.is_dir() {
        for f in fs::read_dir(at)? {
            scan(base, &f?.path(), area, out, cancel)?;
        }
    } else {
        let path = at.strip_prefix(base)?.to_string_lossy().replace('\\', "/");
        if path.ends_with("/shell.lock") || path == "shell.lock" {
            return Ok(());
        }
        if out.len() >= 100_000 {
            return Err("Removal inventory exceeds limit".into());
        }
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = fs::File::open(at)?;
        let mut sha = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Removal preview cancelled".into());
            }
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            sha.update(&buffer[..n]);
        }
        out.push(File {
            area: area.into(),
            path,
            bytes: at.metadata()?.len(),
            sha256: format!("{:x}", sha.finalize()),
        });
    }
    Ok(())
}
pub fn preview(root: &Path, full: bool) -> Result<Plan> {
    preview_with_cancel(root, full, &AtomicBool::new(false))
}
pub fn preview_with_cancel(root: &Path, full: bool, cancel: &AtomicBool) -> Result<Plan> {
    kavychka_storage::preview::no_link_chain(root)?;
    if !root.is_absolute() || fs::read(root.join("install.owner"))? != b"Kavychka.Next.v1\n" {
        return Err("Unknown installation owner".into());
    }
    let c = current(root)?;
    let r = root.canonicalize()?;
    let common = c.common.canonicalize().unwrap_or_else(|_| c.common.clone());
    if kavychka_storage::transfer::owned_tail(&r, &common).is_some()
        || kavychka_storage::transfer::owned_tail(&common, &r).is_some()
        || common.parent().is_none()
    {
        return Err("Unsafe removal roots".into());
    }
    let mut files = vec![];
    for name in ["versions", "updates", "Kavychka.exe"] {
        scan(root, &root.join(name), "program", &mut files, cancel)?;
    }
    let mut external_paths_retained = vec![];
    // The preview owns the SQLite lock; it never reads legacy SQLite directly.
    let data_available = c.data.join("shell.sqlite3").is_file()
        && fs::read(c.data.join("shell-sandbox.owner"))
            .is_ok_and(|b| b == b"kavychka-shell-sandbox-v1\n");
    if full && !data_available {
        return Err("Data unavailable; reconnect its disk. No empty library was created.".into());
    }
    if !data_available && c.common.try_exists()? {
        return Err(
            "Shared data is present but invalid; repair or restore it before removal".into(),
        );
    }
    if data_available {
        let s = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
        let p = s.load()?;
        for value in p.generation.model_paths.values() {
            let path = PathBuf::from(value);
            if kavychka_storage::transfer::owned_tail(&path, &common).is_none() {
                external_paths_retained.push(path);
            }
        }
    } else {
        offline_ready(root)?;
    }
    if let Some(source) = &c.legacy
        && kavychka_storage::transfer::owned_tail(source, &common).is_none()
    {
        external_paths_retained.push(source.clone());
    }
    external_paths_retained.sort();
    external_paths_retained.dedup();
    if full {
        for entry in fs::read_dir(&common)? {
            let name = entry?.file_name().to_string_lossy().to_string();
            if ![
                ".kavychka-next",
                ".gitignore",
                "storage-owner.json",
                "settings.json",
                "library.sqlite3",
                "library.sqlite3-wal",
                "library.sqlite3-shm",
                "library.sqlite3-journal",
                "runtime-ok.json",
                "models",
                "runtime",
                "llama",
                "cosy",
                "audio",
                "presets",
                "profiles",
                "builtin-days",
                "cache",
                "downloads",
                "exports",
                "logs",
                "components",
                "tmp",
                "pip-cache",
            ]
            .contains(&name.as_str())
            {
                return Err("Unrecognized files at shared storage root were retained; move them out or use normal removal".into());
            }
        }
        let marker = common.join("storage-owner.json");
        kavychka_storage::preview::no_link_chain(&marker)?;
        let owned = marker.metadata().is_ok_and(|m| m.len() < 16000)
            && serde_json::from_slice::<serde_json::Value>(&fs::read(marker)?)
                .is_ok_and(|v| v["application"] == "Kavychka");
        if !owned {
            return Err(
                "Full removal requires the shared storage ownership marker; export a profile first"
                    .into(),
            );
        }
        scan(&common, &common, "common", &mut files, cancel)?;
        // Full means recovery data too, and only inside the owned program root.
        scan(root, &root.join("recovery"), "program", &mut files, cancel)?;
    }
    files.sort_by(|a, b| (&a.area, &a.path).cmp(&(&b.area, &b.path)));
    Ok(Plan {
        format: 1,
        program: root.to_owned(),
        common: c.common,
        version: c.version,
        full,
        data_available,
        bytes: files.iter().map(|f| f.bytes).sum(),
        files,
        external_paths_retained,
    })
}
pub fn apply(
    plan: &Plan,
    local: &Path,
    integration: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if plan.format != 1 {
        return Err("Unsupported removal plan".into());
    }
    let _guard = install::owned(&plan.program)?;
    if plan.data_available {
        install::recover(&plan.program)?;
    } else {
        offline_ready(&plan.program)?;
    }
    let checked = preview(&plan.program, plan.full)?;
    if &checked != plan {
        return Err("Installation changed since preview; review removal again".into());
    }
    let c = current(&plan.program)?;
    let locator = locator(local)?;
    if locator
        .as_ref()
        .is_some_and(|l| l.program_root != plan.program || l.common_root != plan.common)
    {
        return Err("Locator belongs to another installation".into());
    }
    let _legacy = if plan.full
        && (plan.common.join("library.sqlite3").exists()
            || plan.common.join("settings.json").exists())
    {
        Some(kavychka_storage::guard::LegacyGuard::acquire()?)
    } else {
        None
    };
    let store = if plan.data_available {
        Some(kavychka_storage::sandbox::Sandbox::open(&c.data)?)
    } else {
        None
    };
    let legacy_pointer = local.join("Kavychka-storage.json");
    kavychka_storage::preview::no_link_chain(&legacy_pointer)?;
    let remove_legacy_pointer =
        plan.full && legacy_pointer.metadata().is_ok_and(|m| m.len() < 16000) && {
            let b = fs::read(&legacy_pointer)?;
            let v: serde_json::Value =
                serde_json::from_slice(b.strip_prefix(&[239, 187, 191]).unwrap_or(&b))?;
            v["directory"].as_str().is_some_and(|p| {
                kavychka_storage::transfer::owned_tail(Path::new(p), &plan.common)
                    .is_some_and(|t| t.as_os_str().is_empty())
            })
        };
    integration(&plan.program)?;
    // Retain a data pointer even after normal removal, for transparent reinstall.
    crate::atomic(&plan.program.join("retained-data.json"), &c)?;
    crate::atomic(&plan.program.join("removal-plan.json"), plan)?;
    if plan.program.join("current.json").exists() {
        crate::windows::replace(
            &plan.program.join("current.json"),
            &plan.program.join("removed-current.json"),
        )?;
    }
    if plan.program.join("transaction.json").exists() {
        let to = plan.program.join("removed-transaction.json");
        crate::windows::replace(&plan.program.join("transaction.json"), &to)?;
    }
    drop(store);
    for name in ["versions", "updates"] {
        let target = plan.program.join(name);
        if target.exists() {
            crate::delete_owned_tree(&target, &plan.program)?;
        }
    }
    let exe = plan.program.join("Kavychka.exe");
    if exe.exists() {
        fs::remove_file(exe)?;
    }
    if plan.full {
        crate::delete_owned_tree(
            &plan.common,
            plan.common.parent().ok_or("Missing storage parent")?,
        )?;
        let recovery = plan.program.join("recovery");
        if recovery.exists() {
            crate::delete_owned_tree(&recovery, &plan.program)?;
        }
        if locator.is_some() {
            fs::remove_file(local.join("Kavychka-next-install.json"))?;
        }
        if remove_legacy_pointer {
            fs::remove_file(legacy_pointer)?;
        }
        for name in [
            "retained-data.json",
            "removed-current.json",
            "removed-transaction.json",
            "removal-plan.json",
        ] {
            let p = plan.program.join(name);
            if p.exists() {
                kavychka_storage::preview::no_link_chain(&p)?;
                fs::remove_file(p)?;
            }
        }
    }
    Ok(())
}
