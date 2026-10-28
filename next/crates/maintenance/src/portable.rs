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
/// Match the exact owned directories across ordinary and verbatim Windows paths.
/// This also works before first installation, when program/data do not exist yet.
/// Ownership and reparse-point checks remain part of the installation transaction.
pub fn matches_layout(root: &Path, program: &Path, common: &Path) -> bool {
    root.is_absolute()
        && program.is_absolute()
        && common.is_absolute()
        && [(program, "program"), (common, "data")]
            .into_iter()
            .all(|(path, directory)| {
                kavychka_storage::transfer::owned_tail(path, &root.join(directory))
                    .is_some_and(|tail| tail.as_os_str().is_empty())
            })
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn exact_layout_accepts_missing_directories_but_rejects_other_targets() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let program = root.join("program");
        let data = root.join("data");
        assert!(!program.exists() && !data.exists());
        assert!(matches_layout(root, &program, &data));
        for wrong in [
            root.join("program-other"),
            program.join("child"),
            root.to_path_buf(),
            PathBuf::from("program"),
        ] {
            assert!(!matches_layout(root, &wrong, &data));
        }
        for wrong in [
            root.join("data-other"),
            data.join("child"),
            root.to_path_buf(),
            PathBuf::from("data"),
        ] {
            assert!(!matches_layout(root, &program, &wrong));
        }
        assert!(!matches_layout(Path::new("."), &program, &data));
    }

    #[cfg(windows)]
    #[test]
    fn old_portable_launcher_context_accepts_both_drive_prefix_forms() {
        let roots = [
            Path::new(r"E:\Кавычка portable"),
            Path::new(r"\\?\E:\Кавычка portable"),
        ];
        let programs = [
            Path::new(r"E:\Кавычка portable\program"),
            Path::new(r"\\?\E:\Кавычка portable\program"),
            Path::new(r"e:\КАВЫЧКА PORTABLE\PROGRAM"),
        ];
        let data = [
            Path::new(r"E:\Кавычка portable\data"),
            Path::new(r"\\?\E:\Кавычка portable\data"),
            Path::new(r"e:\КАВЫЧКА PORTABLE\DATA"),
        ];
        for root in roots {
            for program in programs {
                for common in data {
                    assert!(matches_layout(root, program, common));
                }
            }
            assert!(!matches_layout(
                root,
                Path::new(r"F:\Кавычка portable\program"),
                data[0]
            ));
            assert!(!matches_layout(
                root,
                programs[0],
                Path::new(r"E:\Кавычка portable-other\data")
            ));
        }
    }

    #[cfg(windows)]
    #[test]
    fn unc_layout_accepts_verbatim_prefix_but_not_another_share() {
        let roots = [
            Path::new(r"\\server\share\Кавычка"),
            Path::new(r"\\?\UNC\server\share\Кавычка"),
        ];
        for root in roots {
            for program in [
                Path::new(r"\\SERVER\SHARE\Кавычка\program"),
                Path::new(r"\\?\UNC\server\share\Кавычка\program"),
            ] {
                for common in [
                    Path::new(r"\\server\share\Кавычка\data"),
                    Path::new(r"\\?\UNC\server\share\Кавычка\data"),
                ] {
                    assert!(matches_layout(root, program, common));
                }
                assert!(!matches_layout(
                    root,
                    program,
                    Path::new(r"\\server\share-other\Кавычка\data")
                ));
            }
        }
    }
}
