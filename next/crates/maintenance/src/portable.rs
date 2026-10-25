//! Self-contained portable layout. No system locator, shortcuts or autostart.
use crate::{Result, atomic, install, package};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
const OWNER: &[u8] = b"Kavychka.Next.Portable.v1\n";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    format: u32,
    anchor: PathBuf,
}
pub fn is_portable(root: &Path) -> bool {
    fs::read(root.join("portable.owner")).is_ok_and(|b| b == OWNER)
}
pub fn create(setup: &Path, launcher: &Path, target: &Path) -> Result<()> {
    package::manifest(setup)?;
    kavychka_storage::preview::no_link_chain(target)?;
    if !target.is_absolute() || target.exists() {
        return Err("Choose a new absolute portable folder".into());
    }
    fs::create_dir(target)?;
    fs::write(target.join("portable.owner"), OWNER)?;
    fs::copy(launcher, target.join("Portable.exe"))?;
    fs::copy(setup, target.join("setup.exe"))?;
    fs::write(
        target.join("README.txt"),
        "Run Portable.exe. Program and data stay in this folder. Move the complete folder only while closed. No system shortcuts, locator or autostart are registered. External model paths remain external. Update and rollback use the same verified package mechanism.\r\nЗапускай Portable.exe. Переноси всю папку только при закрытой Кавычке. Внешние модели не копируются.\r\n",
    )?;
    Ok(())
}
fn rebase(value: &mut serde_json::Value, old: &Path, new: &Path) {
    if let Some(map) = value.as_object_mut() {
        for (key, v) in map {
            if [
                "common",
                "data",
                "legacy",
                "backup",
                "program_root",
                "common_root",
                "source",
                "target",
            ]
            .contains(&key.as_str())
            {
                if let Some(path) = v.as_str().map(PathBuf::from)
                    && let Some(tail) = kavychka_storage::transfer::owned_tail(&path, old)
                {
                    *v = serde_json::json!(new.join(tail));
                }
            } else {
                rebase(v, old, new);
            }
        }
    } else if let Some(list) = value.as_array_mut() {
        for v in list {
            rebase(v, old, new);
        }
    }
}
pub fn prepare(
    root: &Path,
    probe: impl Fn(&Path, &install::Current) -> Result<()>,
) -> Result<PathBuf> {
    kavychka_storage::preview::no_link_chain(root)?;
    if !is_portable(root) {
        return Err("Unknown portable owner".into());
    }
    let root = root.canonicalize()?;
    let program = root.join("program");
    let common = root.join("data");
    let state_path = root.join("portable-state.json");
    if state_path.exists() {
        kavychka_storage::preview::no_link_chain(&state_path)?;
        if state_path.metadata()?.len() > 16000 {
            return Err("Portable state exceeds limit".into());
        }
        let state: State = serde_json::from_slice(&fs::read(&state_path)?)?;
        if state.format != 1 || !state.anchor.is_absolute() {
            return Err("Unsupported portable state".into());
        }
        if kavychka_storage::transfer::owned_tail(&root, &state.anchor)
            .is_none_or(|p| !p.as_os_str().is_empty())
        {
            let _guard = install::owned(&program)?;
            // The anchor is committed last; repeating any interrupted step is safe.
            let data = common.join(".kavychka-next");
            if data.join("storage-transfer.json").exists() {
                return Err("Finish the storage transfer before moving the portable folder".into());
            }
            if !data.join("shell.sqlite3").is_file() && program.join("current.json").is_file() {
                return Err("Portable data is missing; no empty replacement created".into());
            }
            if data.join("shell.sqlite3").is_file() {
                let mut s = kavychka_storage::sandbox::Sandbox::open(&data)?;
                s.rebase_storage(&state.anchor, &root)?;
            }
            for name in [
                "current.json",
                "transaction.json",
                "recovery-budget.json",
                "retained-data.json",
            ] {
                let path = program.join(name);
                kavychka_storage::preview::no_link_chain(&path)?;
                if path.exists() {
                    if path.metadata()?.len() > 2_000_000 {
                        return Err("Portable metadata exceeds limit".into());
                    }
                    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
                    rebase(&mut value, &state.anchor, &root);
                    atomic(&path, &value)?;
                }
            }
            let history_path = program.join("storage-paths.json");
            kavychka_storage::preview::no_link_chain(&history_path)?;
            let mut history: Vec<(PathBuf, PathBuf)> = if history_path.exists() {
                if history_path.metadata()?.len() > 64000 {
                    return Err("Portable path history exceeds limit".into());
                }
                serde_json::from_slice(&fs::read(&history_path)?)?
            } else {
                vec![]
            };
            if !history.contains(&(state.anchor.clone(), root.clone())) {
                history.push((state.anchor, root.clone()));
            }
            atomic(&history_path, &history)?;
            atomic(
                &state_path,
                &State {
                    format: 1,
                    anchor: root.clone(),
                },
            )?;
        }
    } else if program.join("current.json").exists() {
        return Err("Portable anchor is missing; restore the complete folder".into());
    }
    if install::current(&program)?.is_none() {
        fs::create_dir_all(&common)?;
        let marker = common.join("storage-owner.json");
        if !marker.exists() {
            atomic(
                &marker,
                &serde_json::json!({"application":"Kavychka","version":1}),
            )?;
        }
        atomic(
            &state_path,
            &State {
                format: 1,
                anchor: root.clone(),
            },
        )?;
        install::apply(&program, &common, &root.join("setup.exe"), probe)?;
    }
    Ok(program)
}
