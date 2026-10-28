//! Independent, data-only component presets; compatible with the old .kavychka ZIP.
use crate::{Error, Result, preview::no_link_chain};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 200 * 1024 * 1024;
mod packs;
pub use packs::{import_collection, save_current_pack};
#[cfg(test)]
#[path = "presets/packs_tests.rs"]
mod packs_tests;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub file: String,
    #[serde(default)]
    pub transcript: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVisual {
    pub file: String,
    #[serde(default = "auto")]
    pub layout: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    Character {
        description: String,
    },
    Voice {
        references: Vec<Reference>,
    },
    Visual {
        file: String,
        #[serde(default = "auto")]
        layout: String,
    },
    /// Self-contained character: no external IDs or paths survive export.
    CharacterPack {
        description: String,
        #[serde(default)]
        references: Vec<Reference>,
        #[serde(default)]
        visual: Option<PackVisual>,
        #[serde(default = "automatic")]
        delivery: String,
        #[serde(default)]
        instruction: String,
    },
}
fn automatic() -> String {
    "auto".into()
}
impl Content {
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Character { description } | Self::CharacterPack { description, .. } => {
                Some(description)
            }
            _ => None,
        }
    }
    pub fn references(&self) -> Option<&[Reference]> {
        match self {
            Self::Voice { references } | Self::CharacterPack { references, .. }
                if !references.is_empty() =>
            {
                Some(references)
            }
            _ => None,
        }
    }
    pub fn visual(&self) -> Option<(&str, &str)> {
        match self {
            Self::Visual { file, layout } => Some((file, layout)),
            Self::CharacterPack {
                visual: Some(v), ..
            } => Some((&v.file, &v.layout)),
            _ => None,
        }
    }
}
fn auto() -> String {
    "auto".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub version: u32,
    pub name: String,
    #[serde(flatten)]
    pub content: Content,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub preset: Preset,
}
fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
pub fn is_reserved_component(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    })
}
fn extension(file: &str, allowed: &[&str]) -> bool {
    Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| allowed.iter().any(|a| a.eq_ignore_ascii_case(e)))
}
pub fn inside(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name.contains(['\\', ':'])
        || name.split('/').any(|c| {
            c.is_empty()
                || c == "."
                || c == ".."
                || c.ends_with(['.', ' '])
                || c.chars().any(char::is_control)
                || is_reserved_component(c)
        })
        || Path::new(name).is_absolute()
    {
        return Err(invalid("invalid preset path"));
    }
    let p = root.join(name);
    no_link_chain(&p)?;
    Ok(p)
}
pub fn folder(root: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || id.len() > 128 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(invalid("invalid preset ID"));
    }
    inside(&root.join("presets"), id)
}
impl Preset {
    pub fn files(&self) -> Vec<&str> {
        match &self.content {
            Content::Character { .. } => vec![],
            Content::Voice { references } => references.iter().map(|r| r.file.as_str()).collect(),
            Content::Visual { file, .. } => vec![file],
            Content::CharacterPack {
                references, visual, ..
            } => {
                let mut files: Vec<_> = references.iter().map(|r| r.file.as_str()).collect();
                if let Some(v) = visual {
                    files.push(v.file.as_str());
                }
                files
            }
        }
    }
    pub fn validate(&self, root: &Path) -> Result<()> {
        if self.version != 1
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid("invalid preset version/name"));
        }
        match &self.content {
            Content::Character { description } => {
                if description.trim().is_empty() || description.chars().count() > 4000 {
                    return Err(invalid("invalid character description"));
                }
            }
            Content::Voice { references } => {
                if references.is_empty()
                    || references.len() > 8
                    || references.iter().any(|r| {
                        r.transcript.chars().count() > 4000 || !extension(&r.file, &["wav", "flac"])
                    })
                {
                    return Err(invalid(
                        "use 1-8 prepared WAV/FLAC references, 1-30 seconds, 16 kHz or higher",
                    ));
                }
            }
            Content::Visual { file, layout } => {
                if !matches!(
                    layout.as_str(),
                    "auto" | "image" | "codex" | "pixel" | "smooth"
                ) || !extension(file, &["png", "jpg", "jpeg", "gif", "webp"])
                {
                    return Err(invalid("invalid visual format/layout"));
                }
            }
            Content::CharacterPack {
                description,
                references,
                visual,
                delivery,
                instruction,
            } => {
                if description.trim().is_empty()
                    || description.chars().count() > 4000
                    || references.len() > 8
                    || references.iter().any(|r| {
                        r.transcript.chars().count() > 4000 || !extension(&r.file, &["wav", "flac"])
                    })
                    || !matches!(delivery.as_str(), "auto" | "custom")
                    || instruction.chars().count() > 600
                    || instruction.contains("<|")
                    || delivery == "custom" && instruction.trim().is_empty()
                    || visual.as_ref().is_some_and(|v| {
                        !matches!(
                            v.layout.as_str(),
                            "auto" | "image" | "codex" | "pixel" | "smooth"
                        ) || !extension(&v.file, &["png", "jpg", "jpeg", "gif", "webp"])
                    })
                {
                    return Err(invalid("invalid complete character preset"));
                }
            }
        }
        let mut total = 0;
        let mut seen = std::collections::BTreeSet::new();
        for file in self.files() {
            if !seen.insert(file.to_lowercase()) {
                return Err(invalid("duplicate preset media path"));
            }
            let path = inside(root, file)?;
            let metadata = path.metadata()?;
            total += metadata.len();
            if !metadata.is_file() || metadata.len() == 0 || total > LIMIT {
                return Err(invalid("preset media missing or exceeds 200 MiB"));
            }
            if self
                .content
                .references()
                .is_some_and(|refs| refs.iter().any(|r| r.file == file))
            {
                audio_header(&path)?;
            }
        }
        Ok(())
    }
}
fn audio_header(path: &Path) -> Result<()> {
    let (rate, frames) = if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("flac"))
    {
        let reader =
            claxon::FlacReader::open(path).map_err(|e| invalid(&format!("invalid FLAC: {e}")))?;
        let info = reader.streaminfo();
        (
            info.sample_rate,
            info.samples
                .ok_or_else(|| invalid("reference needs a known duration"))?,
        )
    } else {
        use std::io::{Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;
        let mut header = [0; 12];
        f.read_exact(&mut header)?;
        if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
            return Err(invalid("reference is not a WAV file"));
        }
        let mut format = None;
        let mut data = None;
        let length = f.metadata()?.len();
        let mut chunks = 0;
        while f.stream_position()? + 8 <= length {
            chunks += 1;
            if chunks > 4096 {
                return Err(invalid("too many WAV metadata chunks"));
            }
            let mut h = [0; 8];
            f.read_exact(&mut h)?;
            let n = u32::from_le_bytes(h[4..].try_into().unwrap()) as u64;
            let start = f.stream_position()?;
            if start + n > length {
                return Err(invalid("truncated WAV reference"));
            }
            if &h[..4] == b"fmt " {
                if n < 16 {
                    return Err(invalid("invalid WAV format"));
                }
                let mut b = [0; 16];
                f.read_exact(&mut b)?;
                let code = u16::from_le_bytes(b[..2].try_into().unwrap());
                let rate = u32::from_le_bytes(b[4..8].try_into().unwrap());
                let align = u16::from_le_bytes(b[12..14].try_into().unwrap());
                if !matches!(code, 1 | 3 | 65534) || align == 0 {
                    return Err(invalid("unsupported prepared WAV encoding"));
                }
                format = Some((rate, align));
            }
            if &h[..4] == b"data" {
                data = Some(n);
            }
            f.seek(SeekFrom::Start(start + n + (n % 2)))?;
        }
        let (rate, align) = format.ok_or_else(|| invalid("WAV format missing"))?;
        (
            rate,
            data.ok_or_else(|| invalid("WAV samples missing"))? / u64::from(align),
        )
    };
    if !(16000..=192000).contains(&rate)
        || frames < u64::from(rate)
        || frames > u64::from(rate) * 30
    {
        return Err(invalid("reference must be 1-30 seconds, 16-192 kHz"));
    }
    Ok(())
}
fn parse(bytes: &[u8]) -> Result<Preset> {
    if bytes.len() > 150000 {
        return Err(invalid("preset manifest too large"));
    }
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes),
    )?)
}
fn read_manifest(path: &Path) -> Result<Preset> {
    no_link_chain(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(150001)
        .read_to_end(&mut bytes)?;
    parse(&bytes)
}
struct PresetStage(PathBuf);
impl Drop for PresetStage {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = remove_tree(&self.0);
        }
    }
}
pub fn read(root: &Path, id: &str) -> Result<Preset> {
    let folder = folder(root, id)?;
    let file = inside(&folder, "preset.json")?;
    let preset = read_manifest(&file)?;
    preset.validate(&folder)?;
    Ok(preset)
}
pub fn list(root: &Path) -> Result<Vec<Entry>> {
    scan(root).map(|(entries, _)| entries)
}
pub fn scan(root: &Path) -> Result<(Vec<Entry>, Vec<String>)> {
    let base = root.join("presets");
    no_link_chain(&base)?;
    if !base.try_exists()? {
        return Ok((vec![], vec![]));
    }
    let mut out = vec![];
    let mut warnings = vec![];
    for f in std::fs::read_dir(base)? {
        let f = f?;
        if f.file_type()?.is_dir() {
            let id = f.file_name().to_string_lossy().into_owned();
            match read(root, &id) {
                Ok(preset) => out.push(Entry { id, preset }),
                Err(error) => warnings.push(format!("{id}: {error}")),
            }
        }
    }
    out.sort_by(|a, b| a.preset.name.cmp(&b.preset.name));
    Ok((out, warnings))
}
fn write(root: &Path, preset: &Preset) -> Result<()> {
    preset.validate(root)?;
    let path = inside(root, "preset.json")?;
    let temp = inside(root, "preset.json.part")?;
    {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(preset)?)?;
        f.sync_all()?;
    }
    crate::guard::atomic_replace(&temp, &path)?;
    Ok(())
}
pub fn update(root: &Path, id: &str, preset: &Preset) -> Result<()> {
    let old = read(root, id)?;
    if std::mem::discriminant(&old.content) != std::mem::discriminant(&preset.content)
        || old.files() != preset.files()
    {
        return Err(invalid("editing cannot replace media paths"));
    }
    write(&folder(root, id)?, preset)
}
pub fn save_character(root: &Path, id: &str, name: String, description: String) -> Result<String> {
    let p = Preset {
        version: 1,
        name,
        content: Content::Character { description },
    };
    if !id.is_empty() {
        update(root, id, &p)?;
        return Ok(id.into());
    }
    import_prepared(root, &p, None)
}
pub fn import_prepared(root: &Path, preset: &Preset, source: Option<&Path>) -> Result<String> {
    preset.validate(source.unwrap_or(root))?;
    let id = format!(
        "p{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("clock error"))?
            .as_nanos()
    );
    let dest = folder(root, &id)?;
    std::fs::create_dir(&dest)?;
    let result = (|| {
        for file in preset.files() {
            let input = inside(source.ok_or_else(|| invalid("missing media source"))?, file)?;
            let output = inside(&dest, file)?;
            std::fs::create_dir_all(output.parent().unwrap())?;
            std::fs::copy(input, output)?;
        }
        write(&dest, preset)?;
        Ok(id)
    })();
    if result.is_err() {
        remove_tree(&dest)?;
    }
    result
}
pub fn import_file(root: &Path, source: &Path) -> Result<String> {
    no_link_chain(source)?;
    if source.metadata()?.len() > LIMIT {
        return Err(invalid("preset exceeds 200 MiB"));
    }
    if source.extension().is_some_and(|e| e == "json") {
        let p = read_manifest(source)?;
        p.validate(
            source
                .parent()
                .ok_or_else(|| invalid("missing source folder"))?,
        )?;
        return import_prepared(root, &p, source.parent());
    }
    // A collection is a ZIP of component .kavychka files, not a profile backup.
    // Inspect by contents so the file name and extension are irrelevant.
    if packs::is_collection(source)? {
        return import_collection(root, source);
    }
    let file = std::fs::File::open(source)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| invalid(&e.to_string()))?;
    if archive.len() > 150 {
        return Err(invalid("too many preset entries"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut size = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| invalid(&e.to_string()))?;
        let name = entry.name().trim_end_matches('/');
        inside(root, name)?;
        if !seen.insert(name.to_lowercase())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(invalid("duplicate or linked archive entry"));
        }
        size = size
            .checked_add(entry.size())
            .ok_or_else(|| invalid("preset size overflow"))?;
        if size > LIMIT {
            return Err(invalid("expanded preset exceeds 200 MiB"));
        }
    }
    let p = {
        let mut f = archive
            .by_name("preset.json")
            .map_err(|_| invalid("missing preset.json"))?;
        if f.size() > 150000 {
            return Err(invalid("manifest too large"));
        }
        let mut b = vec![];
        f.by_ref().take(150001).read_to_end(&mut b)?;
        if b.len() > 150000 || b.len() as u64 != f.size() {
            return Err(invalid(
                "preset manifest expanded beyond its declared size/limit",
            ));
        }
        parse(&b)?
    };
    let stage = root.join("jobs").join(format!(
        "preset-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    no_link_chain(&stage)?;
    std::fs::create_dir(&stage)?;
    let _stage_guard = PresetStage(stage.clone());
    let result = (|| {
        let mut written = 0u64;
        for name in p.files() {
            let target = inside(&stage, name)?;
            let mut f = archive
                .by_name(name)
                .map_err(|_| invalid("missing preset media"))?;
            let declared = f.size();
            if declared > LIMIT.saturating_sub(written) {
                return Err(invalid("preset media too large"));
            }
            std::fs::create_dir_all(target.parent().unwrap())?;
            let mut out = std::fs::File::create(target)?;
            let copied = std::io::copy(&mut f.by_ref().take(declared + 1), &mut out)?;
            written = written.saturating_add(copied);
            if copied != declared || written > LIMIT {
                return Err(invalid(
                    "preset media expanded beyond its declared size/limit",
                ));
            }
        }
        p.validate(&stage)?;
        import_prepared(root, &p, Some(&stage))
    })();
    remove_tree(&stage)?;
    result
}
pub fn export(root: &Path, id: &str, target: &Path) -> Result<()> {
    let p = read(root, id)?;
    no_link_chain(target)?;
    if target.starts_with(root.join("presets")) || target.try_exists()? {
        return Err(invalid("choose a new export file outside presets"));
    }
    let mut zip = zip::ZipWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?,
    );
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let result = (|| {
        zip.start_file("preset.json", options)
            .map_err(|e| invalid(&e.to_string()))?;
        zip.write_all(&serde_json::to_vec_pretty(&p)?)?;
        for name in p.files() {
            zip.start_file(name, options)
                .map_err(|e| invalid(&e.to_string()))?;
            std::io::copy(
                &mut std::fs::File::open(inside(&folder(root, id)?, name)?)?,
                &mut zip,
            )?;
        }
        zip.finish()
            .map_err(|e| invalid(&e.to_string()))?
            .sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(target);
    }
    result
}
pub fn delete(root: &Path, id: &str) -> Result<()> {
    read(root, id)?;
    remove_tree(&folder(root, id)?)
}
fn remove_tree(path: &Path) -> Result<()> {
    no_link_chain(path)?;
    for e in std::fs::read_dir(path)? {
        let e = e?;
        no_link_chain(&e.path())?;
        if e.file_type()?.is_dir() {
            remove_tree(&e.path())?
        } else {
            std::fs::remove_file(e.path())?
        }
    }
    std::fs::remove_dir(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forged_zip64_totals_fail_without_panicking_or_changing_existing_presets() {
        let t = temp();
        let existing = save_character(t.path(), "", "Existing".into(), "Keep me".into()).unwrap();
        let existing_manifest =
            std::fs::read(folder(t.path(), &existing).unwrap().join("preset.json")).unwrap();
        let source = t.path().join("overflow.kavychka");
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("preset.json", zip::write::FileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"{"version":1,"kind":"character","name":"Fixture","description":"Neutral"}"#,
            )
            .unwrap();
        writer
            .start_file("extra.bin", zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(b"x").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let header = bytes
            .windows(4)
            .enumerate()
            .filter(|(_, b)| *b == b"PK\x01\x02")
            .map(|(i, _)| i)
            .nth(1)
            .unwrap();
        let name_len =
            u16::from_le_bytes(bytes[header + 28..header + 30].try_into().unwrap()) as usize;
        assert_eq!(
            u16::from_le_bytes(bytes[header + 30..header + 32].try_into().unwrap()),
            0
        );
        bytes[header + 24..header + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[header + 30..header + 32].copy_from_slice(&12u16.to_le_bytes());
        let mut extra = vec![1, 0, 8, 0];
        extra.extend(u64::MAX.to_le_bytes());
        bytes.splice(header + 46 + name_len..header + 46 + name_len, extra);
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        let old_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
        bytes[end + 12..end + 16].copy_from_slice(&(old_size + 12).to_le_bytes());
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!(archive.by_name("extra.bin").unwrap().size(), u64::MAX);
        std::fs::write(&source, &bytes).unwrap();
        let error = import_file(t.path(), &source).unwrap_err().to_string();
        assert!(error.contains("preset size overflow"), "{error}");
        assert_eq!(list(t.path()).unwrap().len(), 1);
        assert_eq!(
            std::fs::read(folder(t.path(), &existing).unwrap().join("preset.json")).unwrap(),
            existing_manifest
        );
        assert_eq!(std::fs::read_dir(t.path().join("jobs")).unwrap().count(), 0);
        assert_eq!(std::fs::read(source).unwrap(), bytes);
    }
    #[test]
    fn forged_zip_sizes_are_rejected_before_unbounded_read_or_publication() {
        for manifest_bomb in [true, false] {
            let t = temp();
            let source = t.path().join("forged.kavychka");
            let manifest = br#"{"version":1,"kind":"visual","name":"Test","file":"visual.png"}"#;
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            writer
                .start_file("preset.json", zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(manifest).unwrap();
            if manifest_bomb {
                writer.write_all(&vec![b' '; 400000]).unwrap();
            }
            writer
                .start_file("visual.png", zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(&[0; 4096]).unwrap();
            let mut bytes = writer.finish().unwrap().into_inner();
            let target = if manifest_bomb {
                b"preset.json".as_slice()
            } else {
                b"visual.png".as_slice()
            };
            for i in 0..bytes.len() - 46 {
                if &bytes[i..i + 4] == b"PK\x01\x02"
                    && &bytes[i + 46..i + 46 + target.len()] == target
                {
                    bytes[i + 24..i + 28].copy_from_slice(&1u32.to_le_bytes());
                    break;
                }
            }
            std::fs::write(&source, bytes).unwrap();
            let error = import_file(t.path(), &source).unwrap_err().to_string();
            assert!(error.contains("expanded beyond"), "{error}");
            assert_eq!(std::fs::read_dir(t.path().join("jobs")).unwrap().count(), 0);
            assert!(list(t.path()).unwrap().is_empty());
        }
    }
    fn temp() -> tempfile::TempDir {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/preset-tests");
        std::fs::create_dir_all(&p).unwrap();
        let t = tempfile::tempdir_in(p).unwrap();
        std::fs::create_dir(t.path().join("presets")).unwrap();
        std::fs::create_dir(t.path().join("jobs")).unwrap();
        t
    }
    #[test]
    fn character_roundtrip_edit_and_delete_excludes_unknown_secrets() {
        let t = temp();
        let id = save_character(t.path(), "", "Test".into(), "Neutral description".into()).unwrap();
        let zip = t.path().join("export.kavychka");
        export(t.path(), &id, &zip).unwrap();
        let imported = import_file(t.path(), &zip).unwrap();
        assert_ne!(id, imported);
        assert_eq!(read(t.path(), &imported).unwrap().name, "Test");
        save_character(t.path(), &id, "Edited".into(), "New description".into()).unwrap();
        assert_eq!(read(t.path(), &id).unwrap().name, "Edited");
        delete(t.path(), &id).unwrap();
        assert!(read(t.path(), &id).is_err());
        assert_eq!(list(t.path()).unwrap().len(), 1);
        assert!(inside(t.path(), "../escape").is_err());
        assert!(inside(t.path(), "C:/evil").is_err());
        let p=parse(br#"{"version":1,"kind":"character","name":"Test","description":"x","ApiKey":"secret"}"#).unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains("secret"));
    }
    #[test]
    fn zip_traversal_is_rejected_without_creating_destination() {
        let t = temp();
        let zip = t.path().join("bad.kavychka");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        z.start_file("../escape", zip::write::FileOptions::default())
            .unwrap();
        z.write_all(b"bad").unwrap();
        z.finish().unwrap();
        assert!(import_file(t.path(), &zip).is_err());
        assert!(list(t.path()).unwrap().is_empty());
    }
}
