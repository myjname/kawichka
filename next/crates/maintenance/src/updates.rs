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
pub const MANIFEST: &str = "update-manifest.json";
pub const SIGNATURE: &str = "update-manifest.json.minisig";
const MANIFEST_LIMIT: usize = 16 * 1024;
/// Public minisign keys accepted for update manifests. The matching secret keys
/// are kept offline by the maintainer, never on GitHub. The second key is the
/// backup, so a lost primary key does not strand installed copies.
pub const TRUSTED_KEYS: &[&str] = &[
    "RWSzq3pkt4ilTDxJVrhBXoSffVBaB37ZDco2K95dhAA9n/4f1R8QSyTv",
    "RWTp+5F7TBhy0nep3ITxUi6tFr4ActtJEJZKic9HsV9TPVfGpxKzBEyX",
];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedManifest {
    schema: u32,
    version: String,
    asset: String,
    sha256: String,
    bytes: u64,
}
fn has_exact_asset(assets: &[serde_json::Value], tag: &str, name: &str) -> bool {
    let matches: Vec<_> = assets.iter().filter(|a| a["name"] == name).collect();
    matches.len() == 1
        && matches[0]["browser_download_url"].as_str()
            == Some(format!("{PREFIX}next-v{tag}/{name}").as_str())
}
/// The manifest binds version, file name, size and SHA-256 under a key that is
/// not stored on GitHub, so a replaced release asset or edited digest is refused.
pub fn verify_manifest(
    release: &Release,
    manifest: &[u8],
    signature: &[u8],
    keys: &[&str],
) -> Result<()> {
    let signature = minisign_verify::Signature::decode(
        std::str::from_utf8(signature).map_err(|_| "invalid update signature")?,
    )
    .map_err(|_| "invalid update signature")?;
    let trusted = keys.iter().any(|key| {
        minisign_verify::PublicKey::from_base64(key)
            .is_ok_and(|key| key.verify(manifest, &signature, false).is_ok())
    });
    if !trusted {
        return Err("Подпись обновления не подтверждена / Update signature is not trusted".into());
    }
    let signed: SignedManifest = serde_json::from_slice(manifest)?;
    if signed.schema != 1
        || signed.version != release.version
        || signed.asset != ASSET
        || !signed.sha256.eq_ignore_ascii_case(&release.sha256)
        || signed.bytes != release.bytes
    {
        return Err("Signed update manifest does not match this release".into());
    }
    Ok(())
}
fn fetch_small(url: &str, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut command = std::process::Command::new(curl()?);
    command
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "30",
            "--max-filesize",
            &MANIFEST_LIMIT.to_string(),
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    crate::windows::hide(&mut command);
    let mut child = command.spawn()?;
    let mut out = child.stdout.take().ok_or("missing curl output")?;
    let reader = std::thread::spawn(move || {
        let mut b = vec![];
        out.by_ref()
            .take(MANIFEST_LIMIT as u64 + 1)
            .read_to_end(&mut b)
            .map(|_| b)
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
            if !status.success() || b.len() > MANIFEST_LIMIT {
                return Err(
                    "Не удалось получить подпись обновления / Could not fetch update signature"
                        .into(),
                );
            }
            return Ok(b);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
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
        // Releases without a signed manifest are never offered.
        if !has_exact_asset(assets, tag, MANIFEST) || !has_exact_asset(assets, tag, SIGNATURE) {
            continue;
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
/// Fetch and check the signed manifest for a release. The fetcher is injectable so
/// the whole decision (listing, manifest, signature, key) is testable offline.
fn authenticate_release(
    release: &Release,
    cancel: &AtomicBool,
    fetch: &dyn Fn(&str, &AtomicBool) -> Result<Vec<u8>>,
    keys: &[&str],
) -> Result<()> {
    package::version(&release.version)?;
    let base = format!("{PREFIX}next-v{}/", release.version);
    let manifest = fetch(&format!("{base}{MANIFEST}"), cancel)?;
    let signature = fetch(&format!("{base}{SIGNATURE}"), cancel)?;
    verify_manifest(release, &manifest, &signature, keys)
}
pub fn download(release: &Release, root: &Path, cancel: Arc<AtomicBool>) -> Result<PathBuf> {
    authenticate_release(release, &cancel, &fetch_small, TRUSTED_KEYS)?;
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
        let mut v = json!([{"draft":false,"prerelease":false,"tag_name":"v99.0.0","assets":[]},{"draft":false,"prerelease":false,"tag_name":"next-v0.2.0","body":"Release notes","assets":[{"name":ASSET,"browser_download_url":format!("{PREFIX}next-v0.2.0/{ASSET}"),"digest":format!("sha256:{}","a".repeat(64)),"size":5000},{"name":MANIFEST,"browser_download_url":format!("{PREFIX}next-v0.2.0/{MANIFEST}")},{"name":SIGNATURE,"browser_download_url":format!("{PREFIX}next-v0.2.0/{SIGNATURE}")}]}]);
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
    #[test]
    fn release_without_signed_manifest_is_not_offered() {
        let v = json!([{"draft":false,"prerelease":false,"tag_name":"next-v0.2.0","assets":[{"name":ASSET,"browser_download_url":format!("{PREFIX}next-v0.2.0/{ASSET}"),"digest":format!("sha256:{}","a".repeat(64)),"size":5000}]}]);
        assert!(
            parse(&serde_json::to_vec(&v).unwrap(), "0.1.0")
                .unwrap()
                .is_none()
        );
    }
    fn signed(manifest: &str, key: &minisign::KeyPair) -> (Vec<u8>, Vec<u8>) {
        let sig = minisign::sign(
            Some(&key.pk),
            &key.sk,
            std::io::Cursor::new(manifest.as_bytes()),
            None,
            None,
        )
        .unwrap();
        (manifest.as_bytes().to_vec(), sig.to_string().into_bytes())
    }
    fn release() -> Release {
        Release {
            version: "0.2.0".into(),
            url: format!("{PREFIX}next-v0.2.0/{ASSET}"),
            sha256: "ab".repeat(32),
            bytes: 5000,
            notes: String::new(),
        }
    }
    fn manifest_text(version: &str, sha: &str, bytes: u64) -> String {
        format!(
            r#"{{"schema":1,"version":"{version}","asset":"{ASSET}","sha256":"{sha}","bytes":{bytes}}}"#
        )
    }
    #[test]
    fn signed_manifest_must_match_trusted_key_and_release() {
        let good = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = good.pk.to_base64();
        let r = release();
        let (m, s) = signed(&manifest_text("0.2.0", &"ab".repeat(32), 5000), &good);
        verify_manifest(&r, &m, &s, &[&key]).unwrap();
        // Unknown signer.
        assert!(verify_manifest(&r, &m, &s, &[&other.pk.to_base64()]).is_err());
        // Edited manifest after signing.
        let edited = manifest_text("0.2.0", &"cd".repeat(32), 5000);
        assert!(verify_manifest(&r, edited.as_bytes(), &s, &[&key]).is_err());
        // Validly signed, but for another hash, size or version (rollback or swap).
        for text in [
            manifest_text("0.2.0", &"cd".repeat(32), 5000),
            manifest_text("0.2.0", &"ab".repeat(32), 5001),
            manifest_text("0.1.9", &"ab".repeat(32), 5000),
        ] {
            let (m, s) = signed(&text, &good);
            assert!(verify_manifest(&r, &m, &s, &[&key]).is_err());
        }
        // Garbage signature.
        assert!(verify_manifest(&r, &m, b"nonsense", &[&key]).is_err());
    }
    #[test]
    fn embedded_public_keys_are_valid() {
        for key in TRUSTED_KEYS {
            minisign_verify::PublicKey::from_base64(key).unwrap();
        }
    }
    #[test]
    fn repository_key_files_match_the_embedded_keys() {
        let files = [
            include_str!("../../../update-keys/kavychka.pub"),
            include_str!("../../../update-keys/kavychka-backup.pub"),
        ];
        let from_files: Vec<&str> = files
            .iter()
            .map(|f| f.lines().nth(1).unwrap().trim())
            .collect();
        assert_eq!(from_files, TRUSTED_KEYS);
    }
    /// Release listing -> offered release -> signed manifest check, as the app does it.
    #[test]
    fn signed_update_end_to_end_decision() {
        use std::collections::HashMap;
        let key = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pk = key.pk.to_base64();
        let setup = b"installer-bytes".repeat(100);
        let sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&setup))
        };
        let tag = "0.2.0";
        let listing = |digest: &str, with_files: bool| {
            let mut assets = vec![
                json!({"name":ASSET,"browser_download_url":format!("{PREFIX}next-v{tag}/{ASSET}"),"digest":format!("sha256:{digest}"),"size":setup.len()}),
            ];
            if with_files {
                for n in [MANIFEST, SIGNATURE] {
                    assets.push(
                        json!({"name":n,"browser_download_url":format!("{PREFIX}next-v{tag}/{n}")}),
                    );
                }
            }
            serde_json::to_vec(&json!([{"draft":false,"prerelease":false,"tag_name":format!("next-v{tag}"),"assets":assets}])).unwrap()
        };
        // Exactly what next/tools/Sign-release.ps1 writes: ordered keys, compact JSON.
        let manifest = format!(
            r#"{{"schema":1,"version":"{tag}","asset":"{ASSET}","sha256":"{sha}","bytes":{}}}"#,
            setup.len()
        );
        let (m, sig) = signed(&manifest, &key);
        let base = format!("{PREFIX}next-v{tag}/");
        let mut served: HashMap<String, Vec<u8>> = HashMap::new();
        served.insert(format!("{base}{MANIFEST}"), m);
        served.insert(format!("{base}{SIGNATURE}"), sig);
        let fetch = |url: &str, _: &AtomicBool| -> Result<Vec<u8>> {
            served.get(url).cloned().ok_or_else(|| "404".into())
        };
        let cancel = AtomicBool::new(false);
        let keys = [pk.as_str()];
        // Honest release is offered and authenticates.
        let release = parse(&listing(&sha, true), "0.1.0").unwrap().unwrap();
        authenticate_release(&release, &cancel, &fetch, &keys).unwrap();
        // Attacker edits the digest in the GitHub listing: authentication fails.
        let forged = parse(&listing(&"0".repeat(64), true), "0.1.0")
            .unwrap()
            .unwrap();
        assert!(authenticate_release(&forged, &cancel, &fetch, &keys).is_err());
        // Unsigned release is never offered.
        assert!(parse(&listing(&sha, false), "0.1.0").unwrap().is_none());
        // Signature files missing on the server or signed by another key.
        let empty = |_: &str, _: &AtomicBool| -> Result<Vec<u8>> { Err("404".into()) };
        assert!(authenticate_release(&release, &cancel, &empty, &keys).is_err());
        let other = minisign::KeyPair::generate_unencrypted_keypair()
            .unwrap()
            .pk
            .to_base64();
        assert!(authenticate_release(&release, &cancel, &fetch, &[other.as_str()]).is_err());
    }
}
