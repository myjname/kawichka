use crate::{Result, clean_path, digest};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const MAGIC: &[u8; 16] = b"KAVYCHKA-NEXT-01";
pub const BACKEND_FILES: &[&str] = &[
    "backend/server.py",
    "backend/health_check.py",
    "backend/cosy_worker.py",
    "backend/relocate_storage.py",
    "backend/migrate_storage.py",
    "backend/requirements.txt",
    "backend/requirements-cosy.txt",
    "backend/kavychka_local/__init__.py",
    "backend/kavychka_local/character.py",
    "backend/kavychka_local/cosy.py",
    "backend/kavychka_local/delivery.py",
    "backend/kavychka_local/diversity.py",
    "backend/kavychka_local/downloads.py",
    "backend/kavychka_local/engine.py",
    "backend/kavychka_local/hardware.py",
    "backend/kavychka_local/jobs.py",
    "backend/kavychka_local/llama.py",
    "backend/kavychka_local/locale.py",
    "backend/kavychka_local/messages.json",
    "backend/kavychka_local/profiles.py",
    "backend/kavychka_local/store.py",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub platform: String,
    pub files: Vec<Entry>,
}
pub fn version(s: &str) -> Result<(u32, u32, u32)> {
    let p: Vec<_> = s.split('.').collect();
    if p.len() != 3
        || p.iter().any(|s| {
            s.is_empty()
                || s.len() > 8
                || !s.bytes().all(|b| b.is_ascii_digit())
                || s.len() > 1 && s.starts_with('0')
        })
    {
        return Err("expected major.minor.patch".into());
    }
    Ok((p[0].parse()?, p[1].parse()?, p[2].parse()?))
}
fn allowed(name: &str) -> bool {
    matches!(
        name,
        "app.exe"
            | "Kavychka.exe"
            | "next/worker.py"
            | "README.md"
            | "THIRD_PARTY_NOTICES.md"
            | "licenses/Slint-Royalty-free-2.0.md"
            | "licenses/claxon-Apache-2.0.txt"
            | "licenses/zip-MIT.txt"
            | "licenses/byteorder-MIT.txt"
            | "assets/components.json"
            | "assets/character.json"
            | "assets/voice/profile.json"
            | "assets/voice/default-en.wav"
    ) || BACKEND_FILES.contains(&name)
        || name
            .strip_prefix("licenses/dependencies/")
            .is_some_and(|tail| {
                !tail.is_empty()
                    && tail
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_./+-".contains(&b))
            })
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        version(&self.version)?;
        if self.format != 1 || self.platform != "windows-x64" || self.files.len() > 8000 {
            return Err("unsupported/incomplete package".into());
        }
        let mut names = std::collections::BTreeSet::new();
        let mut size = 0;
        for f in &self.files {
            clean_path(Path::new("."), &f.path)?;
            if !allowed(&f.path)
                || !names.insert(f.path.to_lowercase())
                || f.bytes > 200_000_000
                || f.sha256.len() != 64
                || !f.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err("invalid package manifest entry".into());
            }
            size += f.bytes;
        }
        if size > 500_000_000 {
            return Err("package exceeds 500 MB".into());
        }
        for name in [
            "app.exe",
            "Kavychka.exe",
            "next/worker.py",
            "backend/kavychka_local/engine.py",
            "assets/components.json",
            "assets/character.json",
            "assets/voice/profile.json",
        ] {
            if !names.contains(&name.to_lowercase()) {
                return Err(format!("missing package component: {name}").into());
            }
        }
        Ok(())
    }
}
pub fn append(stub: &Path, source: &Path, manifest: Manifest, target: &Path) -> Result<()> {
    manifest.validate()?;
    if target.exists() {
        return Err("package target already exists".into());
    }
    let temporary = target.with_extension("payload.part");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&temporary)?;
    let mut z = zip::ZipWriter::new(file);
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("package.json", options)?;
    z.write_all(&serde_json::to_vec(&manifest)?)?;
    for f in &manifest.files {
        let p = clean_path(source, &f.path)?;
        if p.metadata()?.len() != f.bytes || digest(&p)? != f.sha256 {
            return Err("staged file changed during packaging".into());
        }
        z.start_file(&f.path, options)?;
        std::io::copy(&mut std::fs::File::open(p)?, &mut z)?;
    }
    let zip = z.finish()?;
    zip.sync_all()?;
    drop(zip);
    let bytes = std::fs::metadata(&temporary)?.len();
    let hash = digest(&temporary)?;
    let mut out = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)?;
    std::io::copy(&mut std::fs::File::open(stub)?, &mut out)?;
    std::io::copy(&mut std::fs::File::open(&temporary)?, &mut out)?;
    out.write_all(hash.as_bytes())?;
    out.write_all(&bytes.to_le_bytes())?;
    out.write_all(MAGIC)?;
    out.sync_all()?;
    std::fs::remove_file(temporary)?;
    Ok(())
}
pub fn payload(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len < 88 {
        return Ok(None);
    }
    f.seek(SeekFrom::End(-16))?;
    let mut magic = [0; 16];
    f.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Ok(None);
    }
    f.seek(SeekFrom::End(-24))?;
    let mut n = [0; 8];
    f.read_exact(&mut n)?;
    let size = u64::from_le_bytes(n);
    if size == 0 || size > 500_000_000 || size + 88 > len {
        return Err("invalid installer payload size".into());
    }
    f.seek(SeekFrom::End(-88))?;
    let mut expected = [0; 64];
    f.read_exact(&mut expected)?;
    f.seek(SeekFrom::Start(len - 88 - size))?;
    let mut b = vec![0; size as usize];
    f.read_exact(&mut b)?;
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(&b)).as_bytes() != expected {
        return Err("installer payload integrity check failed".into());
    }
    Ok(Some(b))
}
pub fn manifest(path: &Path) -> Result<Manifest> {
    let b = payload(path)?.ok_or("not a Kavychka Next installer")?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(b))?;
    archive_manifest(&mut z)
}
fn archive_manifest<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Result<Manifest> {
    let mut f = z.by_name("package.json")?;
    const LIMIT: u64 = 2_000_000;
    let declared = f.size();
    if declared > LIMIT {
        return Err("manifest too large".into());
    }
    let mut b = vec![];
    f.by_ref().take(LIMIT + 1).read_to_end(&mut b)?;
    if b.len() as u64 > LIMIT || b.len() as u64 != declared {
        return Err("manifest expanded beyond its declared size or limit".into());
    }
    let m: Manifest = serde_json::from_slice(&b)?;
    m.validate()?;
    Ok(m)
}
pub fn extract(path: &Path, target: &Path) -> Result<Manifest> {
    let b = payload(path)?.ok_or("missing installer payload")?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(b))?;
    if z.len() > 8001 {
        return Err("too many package entries".into());
    }
    // Read the manifest from this verified payload, not a second 500 MB copy
    // reopened from a path that may now refer to a different installer.
    let manifest = archive_manifest(&mut z)?;
    let expected: std::collections::BTreeMap<_, _> = manifest
        .files
        .iter()
        .map(|e| (e.path.as_str(), e))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    if target.exists() {
        return Err("extract target must be new".into());
    }
    std::fs::create_dir(target)?;
    let mut stage = ExtractionStage(Some(target));
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let name = e.name().to_string();
        if !seen.insert(name.to_lowercase())
            || e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("duplicate/linked package entry".into());
        }
        if name == "package.json" {
            continue;
        }
        let wanted = expected
            .get(name.as_str())
            .ok_or("unexpected package entry")?;
        if e.size() != wanted.bytes {
            return Err("package length mismatch".into());
        }
        let p = clean_path(target, &name)?;
        std::fs::create_dir_all(p.parent().unwrap())?;
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&p)?;
        let copied = std::io::copy(&mut e.by_ref().take(wanted.bytes + 1), &mut out)?;
        if copied != wanted.bytes {
            return Err("package expanded beyond its declared size".into());
        }
        out.sync_all()?;
        drop(out);
        if digest(&p)? != wanted.sha256 {
            return Err("package checksum mismatch".into());
        }
    }
    if seen.len() != expected.len() + 1 {
        return Err("package incomplete".into());
    }
    crate::atomic(&target.join("package.json"), &manifest)?;
    stage.0 = None;
    Ok(manifest)
}
struct ExtractionStage<'a>(Option<&'a Path>);
impl Drop for ExtractionStage<'_> {
    fn drop(&mut self) {
        if let Some(path) = self.0
            && kavychka_storage::preview::no_link_chain(path).is_ok()
            && let Some(parent) = path.parent()
        {
            let _ = crate::delete_owned_tree(path, parent);
        }
    }
}
pub fn collect(source: &Path, version: String) -> Result<Manifest> {
    fn walk(p: &Path, root: &Path, out: &mut Vec<Entry>) -> Result<()> {
        for f in std::fs::read_dir(p)? {
            let f = f?;
            kavychka_storage::preview::no_link_chain(&f.path())?;
            if f.file_type()?.is_dir() {
                walk(&f.path(), root, out)?
            } else {
                let name = f
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !allowed(&name) {
                    return Err(format!("unexpected staged file: {name}").into());
                }
                out.push(Entry {
                    path: name,
                    bytes: f.metadata()?.len(),
                    sha256: digest(&f.path())?,
                })
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(source, source, &mut files)?;
    let m = Manifest {
        format: 1,
        version,
        platform: "windows-x64".into(),
        files,
    };
    m.validate()?;
    Ok(m)
}
pub fn verify_installed(root: &Path) -> Result<Manifest> {
    let m: Manifest = serde_json::from_slice(&std::fs::read(root.join("package.json"))?)?;
    m.validate()?;
    for f in &m.files {
        let p = clean_path(root, &f.path)?;
        if p.metadata()?.len() != f.bytes || digest(&p)? != f.sha256 {
            return Err(format!("installed file damaged: {}", f.path).into());
        }
    }
    Ok(m)
}
pub fn app(root: &Path, version: &str) -> Result<PathBuf> {
    self::version(version)?;
    clean_path(root, &format!("versions/{version}/app.exe"))
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn forged_installer(root: &Path, oversized_manifest: bool) -> PathBuf {
        let data = vec![b'x'; 4096];
        let files = [
            "app.exe",
            "Kavychka.exe",
            "next/worker.py",
            "backend/kavychka_local/engine.py",
            "assets/components.json",
            "assets/character.json",
            "assets/voice/profile.json",
            "README.md",
        ];
        let m = Manifest {
            format: 1,
            version: "0.1.25".into(),
            platform: "windows-x64".into(),
            files: files
                .iter()
                .map(|name| Entry {
                    path: (*name).into(),
                    bytes: if *name == "README.md" { 1 } else { 0 },
                    sha256: format!(
                        "{:x}",
                        Sha256::digest(if *name == "README.md" {
                            data.as_slice()
                        } else {
                            &[]
                        })
                    ),
                })
                .collect(),
        };
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("package.json", options).unwrap();
        zip.write_all(&serde_json::to_vec(&m).unwrap()).unwrap();
        if oversized_manifest {
            zip.write_all(&vec![b' '; 2_100_000]).unwrap();
        }
        for name in files {
            zip.start_file(name, options).unwrap();
            if name == "README.md" {
                zip.write_all(&data).unwrap();
            }
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        let target = if oversized_manifest {
            b"package.json".as_slice()
        } else {
            b"README.md".as_slice()
        };
        for i in 0..bytes.len() - 46 {
            if &bytes[i..i + 4] == b"PK\x01\x02"
                && bytes.get(i + 46..i + 46 + target.len()) == Some(target)
            {
                bytes[i + 24..i + 28].copy_from_slice(&1u32.to_le_bytes());
                break;
            }
        }
        let path = root.join("forged.exe");
        let mut out = std::fs::File::create(&path).unwrap();
        out.write_all(b"MZ").unwrap();
        out.write_all(&bytes).unwrap();
        out.write_all(format!("{:x}", Sha256::digest(&bytes)).as_bytes())
            .unwrap();
        out.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
        out.write_all(MAGIC).unwrap();
        path
    }
    #[test]
    fn installer_manifest_actual_size_is_bounded() {
        let t = tempfile::tempdir().unwrap();
        let path = forged_installer(t.path(), true);
        assert!(
            manifest(&path).is_err(),
            "forged manifest expanded past 2 MB and was accepted"
        );
    }
    #[test]
    fn installer_entry_actual_size_is_bounded_and_failed_stage_is_removed() {
        let t = tempfile::tempdir().unwrap();
        let path = forged_installer(t.path(), false);
        let target = t.path().join("stage");
        let result = extract(&path, &target);
        let written = std::fs::metadata(target.join("README.md"))
            .map(|m| m.len())
            .unwrap_or(0);
        println!(
            "forged entry: written={written}, error={:?}",
            result.as_ref().err()
        );
        assert!(
            result.is_err(),
            "4 KB file was accepted with declared bytes=1"
        );
        assert!(!target.exists(), "failed extraction left a partial stage");
    }
}
