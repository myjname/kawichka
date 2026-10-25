//! The data-side fence for an installed, offline shared-storage transfer.
//! The launcher owns pointer recovery. A pending copy is never a live library.
use crate::{Error, Result, datafs, preview::no_link_chain, sandbox::Sandbox};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub format: u32,
    pub operation: String,
    pub state: String,
    pub source: PathBuf,
    pub target: PathBuf,
}
impl Fence {
    pub fn validate(&self) -> Result<()> {
        if self.format != 1
            || self.operation.is_empty()
            || self.operation.len() > 100
            || !self.operation.bytes().all(|b| b.is_ascii_alphanumeric())
            || !["pending", "retired"].contains(&self.state.as_str())
            || !self.source.is_absolute()
            || !self.target.is_absolute()
            || self.source.starts_with(&self.target)
            || self.target.starts_with(&self.source)
        {
            return Err(Error::Invalid("invalid storage transfer fence".into()));
        }
        no_link_chain(&self.source)?;
        no_link_chain(&self.target)
    }
    pub fn write(&self, data: &Path) -> Result<()> {
        self.validate()?;
        if data != self.source && data != self.target {
            return Err(Error::Invalid(
                "transfer fence outside its data roots".into(),
            ));
        }
        datafs::json(&data.join("storage-transfer.json"), self)
    }
}
pub fn fence(data: &Path) -> Result<Option<Fence>> {
    let path = data.join("storage-transfer.json");
    no_link_chain(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    if path.metadata()?.len() > 16000 {
        return Err(Error::Invalid("transfer fence too large".into()));
    }
    let f: Fence = serde_json::from_slice(&std::fs::read(path)?)?;
    f.validate()?;
    if data != f.source && data != f.target {
        return Err(Error::Invalid("misplaced transfer fence".into()));
    }
    Ok(Some(f))
}
pub fn resolve(data: &Path) -> Result<PathBuf> {
    let mut at = data.to_path_buf();
    for _ in 0..16 {
        let Some(f) = fence(&at)? else {
            return Ok(at);
        };
        if f.state != "retired" || at != f.source {
            return Err(Error::Invalid("Перенос хранилища не завершён. Запусти установленную Кавычку для восстановления; пустая библиотека не создавалась. / Storage transfer is pending; start the installed launcher to recover. No empty library was created.".into()));
        }
        if !f.target.is_dir()
            || !f.target.join("shell.sqlite3").is_file()
            || !std::fs::read(f.target.join("shell-sandbox.owner"))
                .is_ok_and(|b| b == b"kavychka-shell-sandbox-v1\n")
        {
            return Err(Error::Invalid("Перенесённое хранилище недоступно. Подключи его диск. / Reconnect the relocated data disk.".into()));
        }
        at = f.target;
    }
    Err(Error::Invalid("too many storage redirects".into()))
}

impl Sandbox {
    /// Keep a read transaction until this offline owner is dropped. Working
    /// stores use rollback journalling; an unexpected WAL mode needs an explicit
    /// repair rather than allowing a concurrent writer behind the transfer.
    pub fn freeze_for_transfer(&self) -> Result<()> {
        let mode: String = self
            .connection
            .pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if !["delete", "truncate", "persist"].contains(&mode.as_str()) {
            return Err(Error::Invalid(
                "unexpected working journal mode; storage transfer not started".into(),
            ));
        }
        self.connection
            .execute_batch("BEGIN; SELECT COUNT(*) FROM preferences;")?;
        Ok(())
    }
    /// A launcher-only probe while the destination is fenced off from normal startup.
    pub fn open_transfer(data: &Path, operation: &str) -> Result<Self> {
        let f = fence(data)?.ok_or_else(|| Error::Invalid("transfer fence missing".into()))?;
        if f.state != "pending" || f.target != data || f.operation != operation {
            return Err(Error::Invalid("transfer probe owner mismatch".into()));
        }
        Self::open_unresolved(data)
    }
    /// Rebase only known operational paths within the copied common root.
    /// Phrase text, independent external paths and archived legacy JSON stay intact.
    pub fn rebase_storage(&mut self, old: &Path, new: &Path) -> Result<()> {
        fn moved(p: &Path, old: &Path, new: &Path) -> PathBuf {
            owned_tail(p, old)
                .map(|tail| new.join(tail))
                .unwrap_or_else(|| p.to_owned())
        }
        let mut prefs = self.load()?;
        for path in prefs.generation.model_paths.values_mut() {
            *path = moved(Path::new(path), old, new)
                .to_string_lossy()
                .into_owned();
        }
        prefs.validate().map_err(Error::Invalid)?;
        let origin = self.origin()?.map(|mut o| {
            o.source = moved(&o.source, old, new);
            o.snapshot = moved(&o.snapshot, old, new);
            o
        });
        let recovery = self
            .meta("recovery_path")?
            .map(|value| {
                value
                    .as_str()
                    .map(|s| moved(Path::new(s), old, new))
                    .ok_or_else(|| Error::Invalid("invalid recovery path".into()))
            })
            .transpose()?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE preferences SET json=?1 WHERE id=1",
            [serde_json::to_string(&prefs)?],
        )?;
        for (key, value) in [
            ("origin", origin.map(serde_json::to_value).transpose()?),
            (
                "recovery_path",
                recovery.map(serde_json::to_value).transpose()?,
            ),
        ] {
            if let Some(value) = value {
                tx.execute(
                    "UPDATE library_meta SET json=?1 WHERE key=?2",
                    rusqlite::params![serde_json::to_string(&value)?, key],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

// Origin paths come from canonicalize() (\\?\ prefix on Windows), whereas the
// installation pointer may be a normal drive path. Compare normalized absolute
// components; keep the tail's spelling and never perform a text substring replace.
pub fn owned_tail(path: &Path, root: &Path) -> Option<PathBuf> {
    #[cfg(not(windows))]
    {
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    }
    #[cfg(windows)]
    {
        fn parts(path: &Path) -> Option<Vec<String>> {
            let text = path.to_str()?.replace('/', "\\");
            let text = if text
                .get(..8)
                .is_some_and(|p| p.eq_ignore_ascii_case("\\\\?\\UNC\\"))
            {
                format!("\\\\{}", &text[8..])
            } else if let Some(tail) = text.strip_prefix("\\\\?\\") {
                tail.to_owned()
            } else {
                text
            };
            let p = Path::new(&text);
            if !p.is_absolute() {
                return None;
            }
            let mut result = vec![];
            for part in p.components() {
                match part {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        if result.len() <= 2 {
                            return None;
                        }
                        result.pop();
                    }
                    _ => result.push(part.as_os_str().to_str()?.to_owned()),
                }
            }
            Some(result)
        }
        let path = parts(path)?;
        let root = parts(root)?;
        if path.len() < root.len()
            || path
                .iter()
                .zip(&root)
                .any(|(a, b)| a.to_lowercase() != b.to_lowercase())
        {
            return None;
        }
        Some(path[root.len()..].iter().collect())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn operational_paths_handle_canonical_prefix_case_unicode_and_component_boundaries() {
        let old = Path::new("E:\\Общая папка");
        let next = Path::new("F:\\Новая папка");
        for path in [
            "\\\\?\\E:\\Общая папка\\.kavychka-next\\legacy",
            "e:\\ОБЩАЯ ПАПКА\\models\\owned.gguf",
            "E:\\Общая папка\\nested\\..\\models\\owned.gguf",
        ] {
            let tail = owned_tail(Path::new(path), old).unwrap();
            assert!(next.join(tail).starts_with(next));
        }
        assert_eq!(
            owned_tail(Path::new("\\\\?\\E:\\Общая папка\\models\\owned.gguf"), old).unwrap(),
            Path::new("models/owned.gguf")
        );
        assert!(owned_tail(Path::new("E:\\Общая папка другая\\model.gguf"), old).is_none());
        assert!(owned_tail(Path::new("E:\\Общая папка\\..\\external.gguf"), old).is_none());
        assert_eq!(
            owned_tail(
                Path::new("\\\\?\\UNC\\server\\share\\common\\models"),
                Path::new("\\\\SERVER\\share\\common")
            )
            .unwrap(),
            Path::new("models")
        );
    }
}
