//! Minimal pinned Python bootstrap, only for an explicit component installation.
use kavychka_maintenance::{Result, clean_path, digest};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub fn ensure_python(app: &Path, library: &Path, progress: impl FnMut(&str)) -> Result<PathBuf> {
    ensure_python_with_repair(app, library, false, progress)
}
fn ensure_python_with_repair(
    app: &Path,
    library: &Path,
    repair: bool,
    mut progress: impl FnMut(&str),
) -> Result<PathBuf> {
    let managed = library.join("components");
    kavychka_storage::preview::no_link_chain(&managed)?;
    std::fs::create_dir_all(&managed)?;
    let runtime = managed.join("runtime");
    kavychka_storage::preview::no_link_chain(&runtime)?;
    // The completion record moves together with the verified interpreter. A crash
    // between publishing the directory and writing a separate record cannot strand it.
    let marker = runtime.join("python-bootstrap.json");
    if runtime.join("python.exe").is_file() && marker.is_file() {
        let v = std::fs::read(&marker)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        if v.as_ref().is_some_and(|v| {
            digest(&runtime.join("python.exe")).is_ok_and(|sha| v["exe_sha256"] == sha)
                && digest(&runtime.join("python312._pth")).is_ok_and(|sha| v["pth_sha256"] == sha)
        }) {
            return Ok(runtime.join("python.exe"));
        }
    }
    if runtime.exists() {
        if !repair {
            return Err("Управляемая среда Python требует восстановления. Нажмите «Подготовить модели». / Managed Python needs recovery; choose Set up models.".into());
        }
        let managed_absolute = managed.canonicalize()?;
        let source = runtime.canonicalize()?;
        let retained = managed_absolute.join(format!(
            "runtime-unverified-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        if source.parent() != Some(managed_absolute.as_path())
            || retained.parent() != Some(managed_absolute.as_path())
            || retained.exists()
        {
            return Err("Unsafe runtime repair paths".into());
        }
        std::fs::rename(&source, &retained)?;
        progress(
            "Повреждённая управляемая среда сохранена рядом; устанавливаю новую / Unverified managed environment retained beside the new runtime",
        );
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(app.join("assets/components.json"))?)?;
    let spec = &manifest["python"];
    let url = spec["url"].as_str().ok_or("Python URL missing")?;
    let bytes = spec["size"].as_u64().ok_or("Python size missing")?;
    let hash = spec["sha256"].as_str().ok_or("Python hash missing")?;
    if !url.starts_with("https://www.python.org/ftp/python/")
        || !url.ends_with("-embed-amd64.zip")
        || bytes > 30_000_000
        || hash.len() != 64
    {
        return Err("invalid pinned Python descriptor".into());
    }
    kavychka_maintenance::windows::disk_space(&managed, bytes * 4 + 50_000_000)?;
    let zip = managed.join("python.zip.part");
    kavychka_storage::preview::no_link_chain(&zip)?;
    let valid = zip.is_file() && zip.metadata()?.len() == bytes && digest(&zip)? == hash;
    if !valid {
        if zip.exists() && zip.metadata()?.len() >= bytes {
            std::fs::remove_file(&zip)?;
        }
        progress("Python · 11 MB · загрузка с python.org / downloading from python.org");
        let curl = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?)
            .join("System32/curl.exe");
        let mut command = std::process::Command::new(curl);
        command
            .args([
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "300",
                "--continue-at",
                "-",
                "--max-filesize",
                &bytes.to_string(),
                "--output",
            ])
            .arg(&zip)
            .arg(url);
        kavychka_maintenance::windows::hide(&mut command);
        if !command.status()?.success() {
            return Err("Python download interrupted; retry to resume".into());
        }
        if zip.metadata()?.len() != bytes || digest(&zip)? != hash {
            std::fs::remove_file(&zip)?;
            return Err("Python ZIP integrity check failed".into());
        }
    }
    progress("Python · проверка и распаковка / verifying and extracting");
    let stage = managed.join("runtime-stage");
    if stage.exists() {
        kavychka_maintenance::delete_owned_tree(&stage, &managed)?;
    }
    std::fs::create_dir(&stage)?;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip)?)?;
    if archive.len() > 1000 {
        return Err("too many Python files".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().trim_end_matches('/').to_string();
        let target = clean_path(&stage, &name)?;
        if !seen.insert(name.to_lowercase())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("invalid Python archive entry".into());
        }
        total += entry.size();
        if total > 100_000_000 {
            return Err("Python archive exceeds limit".into());
        }
        if entry.is_dir() {
            std::fs::create_dir_all(target)?
        } else {
            std::fs::create_dir_all(target.parent().unwrap())?;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            std::io::copy(&mut entry, &mut f)?;
            f.sync_all()?;
        }
    }
    std::fs::create_dir_all(stage.join("Lib/site-packages"))?;
    std::fs::write(
        stage.join("python312._pth"),
        b"python312.zip\r\n.\r\nLib/site-packages\r\nimport site\r\n",
    )?;
    let record = serde_json::json!({"format":1,"archive_sha256":hash,"exe_sha256":digest(&stage.join("python.exe"))?,"pth_sha256":digest(&stage.join("python312._pth"))?});
    kavychka_maintenance::atomic(&stage.join("python-bootstrap.json"), &record)?;
    std::fs::rename(stage, &runtime)?;
    std::fs::remove_file(zip)?;
    Ok(runtime.join("python.exe"))
}
pub fn worker() -> Result<()> {
    let mut bytes = String::new();
    std::io::stdin()
        .take(4_000_001)
        .read_to_string(&mut bytes)?;
    if bytes.len() > 4_000_000 {
        return Err("bootstrap request exceeds limit".into());
    }
    let request: serde_json::Value = serde_json::from_str(&bytes)?;
    let library = PathBuf::from(
        request["library_root"]
            .as_str()
            .ok_or("missing library root")?,
    );
    let job = PathBuf::from(request["work_dir"].as_str().ok_or("missing job root")?);
    if job.parent() != Some(library.join("jobs").as_path())
        || !job
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("job-"))
        || request["operation"] != "install"
    {
        return Err("bootstrap is only allowed for an owned installation task".into());
    }
    kavychka_storage::preview::no_link_chain(&job)?;
    if std::fs::read(library.join("shell-sandbox.owner"))? != b"kavychka-shell-sandbox-v1\n" {
        return Err("unknown data owner".into());
    }
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or("program path missing")?;
    let app = if dir.join("next/worker.py").is_file() {
        dir.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()?
    };
    let task = request["task"].as_str().ok_or("task ID missing")?;
    let python = ensure_python_with_repair(
        &app,
        &library,
        // This is already an explicit installation request in an owned data
        // root. Retain an unverified managed runtime and recover automatically;
        // the old source environment and all model weights remain untouched.
        true,
        |message| {
            println!(
                "{}",
                serde_json::json!({"version":1,"task":task,"kind":"progress","phase":"install_runtime","message":message,"done":0,"total":0,"percent":null})
            )
        },
    );
    let python = match python {
        Ok(python) => python,
        Err(error) => {
            eprintln!("Python bootstrap failed: {error}");
            println!(
                "{}",
                serde_json::json!({"version":1,"task":task,"kind":"error","phase":"install_runtime","message":format!("Не удалось подготовить Python для моделей: {error}. Повторите «Подготовить модели» / Could not prepare Python; retry Set up models"),"detail":error.to_string()})
            );
            return Ok(());
        }
    };
    let mut cmd = std::process::Command::new(python);
    cmd.arg("-B")
        .arg(app.join("next/worker.py"))
        .current_dir(&app)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::piped())
        .env("TEMP", &job)
        .env("TMP", &job);
    kavychka_maintenance::windows::hide(&mut cmd);
    let mut child = cmd.spawn()?;
    {
        let mut input = child.stdin.take().ok_or("missing Python input")?;
        input.write_all(bytes.as_bytes())?;
        input.flush()?;
    }
    if !child.wait()?.success() {
        return Err("component installation worker failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_repair_retains_unverified_runtime_even_when_bootstrap_cannot_continue() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../work/runtime-repair-tests");
        std::fs::create_dir_all(&base).unwrap();
        let t = tempfile::tempdir_in(base).unwrap();
        let runtime = t.path().join("data/components/runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join("python.exe"), b"unverified-runtime").unwrap();
        std::fs::write(runtime.join("python-bootstrap.json"), b"malformed").unwrap();
        assert!(
            super::ensure_python_with_repair(t.path(), &t.path().join("data"), false, |_| {})
                .is_err()
        );
        assert!(runtime.exists());
        assert!(
            super::ensure_python_with_repair(t.path(), &t.path().join("data"), true, |_| {})
                .is_err()
        );
        assert!(!runtime.exists());
        let retained = std::fs::read_dir(t.path().join("data/components"))
            .unwrap()
            .map(|f| f.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("runtime-unverified-")
            })
            .unwrap();
        assert_eq!(
            std::fs::read(retained.join("python.exe")).unwrap(),
            b"unverified-runtime"
        );
    }
}
