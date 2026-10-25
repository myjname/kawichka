//! Fixed public GitHub origin. No tokens, settings, phrases or telemetry sent.
use crate::{Result, package};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub const API: &str = "https://api.github.com/repos/myjname/kawichka/releases?per_page=30";
const PREFIX: &str = "https://github.com/myjname/kawichka/releases/download/";
pub const ASSET: &str = "Kavychka-Next-Setup.exe";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub notes: String,
}
pub fn parse(bytes: &[u8], current: &str) -> Result<Option<Release>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let list = value.as_array().ok_or("invalid release listing")?;
    let current = package::version(current)?;
    let mut releases = vec![];
    for item in list {
        if item["draft"] != false || item["prerelease"] != false {
            continue;
        }
        let Some(tag) = item["tag_name"]
            .as_str()
            .and_then(|s| s.strip_prefix("next-v"))
        else {
            continue;
        };
        let ver = package::version(tag)?;
        if ver <= current {
            continue;
        }
        let assets = item["assets"].as_array().ok_or("missing release assets")?;
        let matches: Vec<_> = assets.iter().filter(|a| a["name"] == ASSET).collect();
        if matches.is_empty() {
            continue;
        }
        if matches.len() != 1 {
            return Err("duplicate setup asset".into());
        }
        let a = matches[0];
        let url = a["browser_download_url"]
            .as_str()
            .ok_or("missing update URL")?;
        let expected = format!("{PREFIX}next-v{tag}/{ASSET}");
        if url != expected {
            return Err("unexpected update origin".into());
        }
        let hash = a["digest"]
            .as_str()
            .and_then(|s| s.strip_prefix("sha256:"))
            .ok_or("release asset has no SHA-256")?;
        let size = a["size"]
            .as_u64()
            .filter(|n| (1000..=500_000_000).contains(n))
            .ok_or("invalid update size")?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid update digest".into());
        }
        releases.push((
            ver,
            Release {
                version: tag.into(),
                url: url.into(),
                sha256: hash.to_lowercase(),
                bytes: size,
                notes: item["body"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(8000)
                    .collect(),
            },
        ));
    }
    releases.sort_by_key(|(v, _)| *v);
    Ok(releases.pop().map(|(_, r)| r))
}
fn curl() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?)
            .join("System32/curl.exe"),
    )
}
pub fn check(current: &str, cancel: &AtomicBool) -> Result<Option<Release>> {
    check_with_ca(current, cancel, None)
}
// A private per-request CA seam is used only by the HTTPS qualification test.
// The application always uses Windows trust and the fixed public origins.
fn check_with_ca(
    current: &str,
    cancel: &AtomicBool,
    test_ca: Option<&Path>,
) -> Result<Option<Release>> {
    let mut command = std::process::Command::new(curl()?);
    command.arg("--disable");
    if let Some(ca) = test_ca {
        command.arg("--cacert").arg(ca);
    }
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-time",
            "15",
            "--max-filesize",
            "2000000",
            "--header",
            "Accept: application/vnd.github+json",
            "--user-agent",
            "Kavychka-Next",
            API,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    crate::windows::hide(&mut command);
    let mut child = command.spawn()?;
    let mut out = child.stdout.take().ok_or("missing curl output")?;
    let reader = std::thread::spawn(move || {
        let mut b = vec![];
        out.by_ref().take(2_000_001).read_to_end(&mut b).map(|_| b)
    });
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("update check cancelled".into());
        }
        if let Some(status) = child.try_wait()? {
            let b = reader.join().map_err(|_| "HTTP reader failed")??;
            if !status.success() || b.len() > 2_000_000 {
                return Err("Не удалось проверить обновления / Could not check releases".into());
            }
            return parse(&b, current);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
pub fn download(release: &Release, root: &Path, cancel: Arc<AtomicBool>) -> Result<PathBuf> {
    download_with_ca(release, root, cancel, None)
}
fn download_with_ca(
    release: &Release,
    root: &Path,
    cancel: Arc<AtomicBool>,
    test_ca: Option<&Path>,
) -> Result<PathBuf> {
    package::version(&release.version)?;
    if release.url != format!("{PREFIX}next-v{}/{ASSET}", release.version)
        || release.sha256.len() != 64
        || !release.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || !(1000..=500_000_000).contains(&release.bytes)
    {
        return Err("invalid update descriptor".into());
    }
    let folder = root.join("updates");
    kavychka_storage::preview::no_link_chain(&folder)?;
    std::fs::create_dir_all(&folder)?;
    crate::windows::disk_space(
        &folder,
        release
            .bytes
            .saturating_mul(3)
            .saturating_add(64 * 1024 * 1024),
    )?;
    let target = folder.join(format!("{}-{ASSET}", release.version));
    kavychka_storage::preview::no_link_chain(&target)?;
    let verified = |p: &Path| -> Result<bool> {
        Ok(p.is_file()
            && p.metadata()?.len() == release.bytes
            && crate::digest(p)? == release.sha256)
    };
    if verified(&target)? {
        return Ok(target);
    }
    let partial = target.with_extension("part");
    kavychka_storage::preview::no_link_chain(&partial)?;
    if partial.exists() && partial.metadata()?.len() > release.bytes {
        std::fs::remove_file(&partial)?
    }
    let mut cmd = std::process::Command::new(curl()?);
    cmd.arg("--disable");
    if let Some(ca) = test_ca {
        cmd.arg("--cacert").arg(ca);
    }
    cmd.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "15",
        "--max-time",
        "600",
        "--max-filesize",
        &release.bytes.to_string(),
        "--continue-at",
        "-",
        "--output",
    ])
    .arg(&partial)
    .arg(&release.url)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    crate::windows::hide(&mut cmd);
    let mut child = cmd.spawn()?;
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("update download cancelled; partial file retained".into());
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(
                    "Загрузка прервана; можно повторить / Download interrupted; retry to resume"
                        .into(),
                );
            }
            break;
        }
        if partial.metadata().is_ok_and(|m| m.len() > release.bytes) {
            let _ = child.kill();
            let _ = child.wait();
            std::fs::remove_file(&partial)?;
            return Err("update exceeds expected size".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !verified(&partial)? {
        std::fs::remove_file(&partial)?;
        return Err("Update SHA-256 mismatch; installed version unchanged".into());
    }
    let manifest = package::manifest(&partial)?;
    if manifest.version != release.version {
        return Err("update manifest version mismatch".into());
    }
    if target.exists() {
        std::fs::remove_file(&target)?
    }
    std::fs::rename(&partial, &target)?;
    Ok(target)
}
/// Recheck at execution time and keep a read handle denying replacement on Windows.
pub fn executable_guard(release: &Release, path: &Path) -> Result<std::fs::File> {
    kavychka_storage::preview::no_link_chain(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let handle = options.open(path)?;
    if handle.metadata()?.len() != release.bytes
        || crate::digest(path)? != release.sha256
        || package::manifest(path)?.version != release.version
    {
        return Err(
            "Downloaded update changed; check and download again. Installed version retained."
                .into(),
        );
    }
    Ok(handle)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    #[ignore = "Explicit local HTTPS fixture; no machine certificate store changes"]
    fn real_https_cancel_resume_digest_and_execution_guard() {
        run_https().unwrap();
    }
    fn run_https() -> Result<()> {
        use std::time::{Duration, Instant};
        let outer = PathBuf::from(
            std::env::var_os("KAVYCHKA_UPDATE_TLS_FIXTURE").ok_or("Set local fixture root")?,
        );
        let root = outer.join("client");
        let ca = outer.join("certificate.pem");
        if !outer.is_absolute() || root.exists() {
            return Err("New absolute fixture root required".into());
        }
        std::fs::create_dir(&root)?;
        let release = check_with_ca("0.1.2", &AtomicBool::new(false), Some(&ca))?
            .ok_or("Missing fixture release")?;
        let partial = root
            .join("updates")
            .join(format!("{}-Kavychka-Next-Setup.part", release.version));
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let path = partial.clone();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let watcher = std::thread::spawn(move || {
            let started = Instant::now();
            while !finished.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(30) {
                if path.metadata().is_ok_and(|m| m.len() > 1024 * 1024) {
                    token.store(true, Ordering::Release);
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let stopped = download_with_ca(&release, &root, cancel.clone(), Some(&ca));
        done.store(true, Ordering::Release);
        let _ = watcher.join();
        if stopped.is_ok() || !cancel.load(Ordering::Acquire) {
            return Err("Actual download cancellation was not observed".into());
        }
        let bytes = partial.metadata()?.len();
        if bytes == 0 || bytes >= release.bytes {
            return Err("No resumable partial".into());
        }
        let resumed =
            download_with_ca(&release, &root, Arc::new(AtomicBool::new(false)), Some(&ca))?;
        let guard = executable_guard(&release, &resumed)?;
        #[cfg(windows)]
        if std::fs::OpenOptions::new()
            .write(true)
            .open(&resumed)
            .is_ok()
        {
            return Err("Guard did not deny overwrite".into());
        }
        drop(guard);
        let bad = check_with_ca("0.1.2", &AtomicBool::new(false), Some(&ca))?
            .ok_or("Missing corrupt fixture release")?;
        if bad.version == release.version {
            return Err("Fixture did not advance".into());
        }
        let error = download_with_ca(&bad, &root, Arc::new(AtomicBool::new(false)), Some(&ca))
            .expect_err("Wrong digest must fail");
        if !error.to_string().contains("SHA-256 mismatch") {
            return Err(error);
        }
        std::fs::write(
            root.join("result.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":true,"actual_windows_curl_https":true,"fixed_origins_unchanged":true,"test_ca_per_request_only":true,"system_certificate_store_changed":false,"cancelled_partial_bytes":bytes,"resume_sha_and_manifest_verified":true,"execution_guard_denies_writes":true,"wrong_digest_refused":true,"installer_executed":false,"real_public_release_channel_tested":false}),
            )?,
        )?;
        Ok(())
    }
    #[test]
    fn releases_require_rust_channel_exact_origin_and_digest() {
        let mut v = json!([{"draft":false,"prerelease":false,"tag_name":"v99.0.0","assets":[]},{"draft":false,"prerelease":false,"tag_name":"next-v0.2.0","body":"Release notes","assets":[{"name":ASSET,"browser_download_url":format!("{PREFIX}next-v0.2.0/{ASSET}"),"digest":format!("sha256:{}","a".repeat(64)),"size":5000}]}]);
        assert_eq!(
            parse(&serde_json::to_vec(&v).unwrap(), "0.1.0")
                .unwrap()
                .unwrap()
                .version,
            "0.2.0"
        );
        assert!(
            parse(&serde_json::to_vec(&v).unwrap(), "0.2.0")
                .unwrap()
                .is_none()
        );
        v[1]["assets"][0]["browser_download_url"] = json!("https://evil.invalid/setup.exe");
        assert!(parse(&serde_json::to_vec(&v).unwrap(), "0.1.0").is_err());
    }
}
