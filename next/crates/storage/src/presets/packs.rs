//! Complete, self-contained characters assembled from independent presets.
use super::*;
use kavychka_core::preferences::Preferences;

fn stage(root: &Path) -> Result<PresetStage> {
    let base = root.join("jobs");
    no_link_chain(&base)?;
    std::fs::create_dir_all(&base)?;
    let path = base.join(format!(
        "character-{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("clock error"))?
            .as_nanos()
    ));
    std::fs::create_dir(&path)?;
    Ok(PresetStage(path))
}

/// Captures only the chosen character components, never hardware/models/schedule.
pub fn save_current_pack(root: &Path, name: String, prefs: &Preferences) -> Result<String> {
    let temporary = stage(root)?;
    let mut references = vec![];
    let mut visual = None;
    if !prefs.generation.voice_profile.is_empty() {
        let preset = read(root, &prefs.generation.voice_profile)?;
        let refs = preset
            .content
            .references()
            .ok_or_else(|| invalid("selected voice preset has no references"))?;
        for (i, r) in refs.iter().enumerate() {
            let file = format!(
                "voice{i}.{}",
                Path::new(&r.file)
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("wav")
                    .to_ascii_lowercase()
            );
            std::fs::copy(
                inside(&folder(root, &prefs.generation.voice_profile)?, &r.file)?,
                temporary.0.join(&file),
            )?;
            references.push(Reference {
                file,
                transcript: r.transcript.clone(),
            });
        }
    }
    if !prefs.desktop.visual_preset.is_empty() {
        let preset = read(root, &prefs.desktop.visual_preset)?;
        let (original, layout) = preset
            .content
            .visual()
            .ok_or_else(|| invalid("selected appearance preset has no image"))?;
        let file = format!(
            "visual.{}",
            Path::new(original)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase()
        );
        std::fs::copy(
            inside(&folder(root, &prefs.desktop.visual_preset)?, original)?,
            temporary.0.join(&file),
        )?;
        visual = Some(PackVisual {
            file,
            layout: layout.into(),
        });
    }
    let (delivery, instruction) = match prefs.generation.cosy_style.as_str() {
        "auto" => ("auto".into(), String::new()),
        "custom" => ("custom".into(), prefs.generation.cosy_instruction.clone()),
        chosen => {
            let text = prefs
                .generation
                .delivery_catalog
                .as_array()
                .and_then(|items| items.iter().find(|v| v["id"] == chosen))
                .and_then(|v| v["instruction"].as_str())
                .ok_or_else(|| invalid("selected delivery is unavailable"))?;
            ("custom".into(), text.into())
        }
    };
    let p = Preset {
        version: 1,
        name,
        content: Content::CharacterPack {
            description: prefs.generation.character_description.clone(),
            references,
            visual,
            delivery,
            instruction,
        },
    };
    import_prepared(root, &p, Some(&temporary.0))
}

fn checked_archive(archive: &mut zip::ZipArchive<std::fs::File>, root: &Path) -> Result<()> {
    if archive.len() > 150 {
        return Err(invalid("too many character collection entries"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut size = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| invalid(&e.to_string()))?;
        inside(root, entry.name().trim_end_matches('/'))?;
        if !seen.insert(entry.name().to_lowercase())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(invalid("duplicate or linked character collection entry"));
        }
        size = size
            .checked_add(entry.size())
            .ok_or_else(|| invalid("character collection size overflow"))?;
        if size > LIMIT {
            return Err(invalid("expanded character collection exceeds 200 MiB"));
        }
    }
    Ok(())
}
pub(super) fn is_collection(source: &Path) -> Result<bool> {
    let mut zip =
        zip::ZipArchive::new(std::fs::File::open(source)?).map_err(|e| invalid(&e.to_string()))?;
    if zip.by_name("preset.json").is_ok() {
        return Ok(false);
    }
    Ok(zip.file_names().any(|n| {
        Path::new(n)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("kavychka"))
    }))
}

/// Old downloadable collections with 1–3 nested component presets are accepted
/// atomically. Readme files are ignored; only typed preset media are published.
pub fn import_collection(root: &Path, source: &Path) -> Result<String> {
    no_link_chain(source)?;
    if source.metadata()?.len() > LIMIT {
        return Err(invalid("character collection exceeds 200 MiB"));
    }
    let temporary = stage(root)?;
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(source)?).map_err(|e| invalid(&e.to_string()))?;
    checked_archive(&mut archive, &temporary.0)?;
    std::fs::create_dir(temporary.0.join("presets"))?;
    std::fs::create_dir(temporary.0.join("jobs"))?;
    let mut character = None;
    let mut voice = None;
    let mut visual = None;
    let mut count = 0;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| invalid(&e.to_string()))?;
        if entry.is_dir()
            || !Path::new(entry.name())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("kavychka"))
        {
            continue;
        }
        count += 1;
        if count > 3 {
            return Err(invalid(
                "use one character, one voice and one appearance in a collection",
            ));
        }
        let target = temporary.0.join(format!("input{count}.kavychka"));
        let declared = entry.size();
        let mut output = std::fs::File::create(&target)?;
        let copied = std::io::copy(
            &mut entry.by_ref().take(declared.saturating_add(1)),
            &mut output,
        )?;
        drop(output);
        total = total.saturating_add(copied);
        if copied != declared || total > LIMIT {
            return Err(invalid(
                "character collection expanded beyond its declared size/limit",
            ));
        }
        // Collections of collections are intentionally rejected (bounded depth).
        let mut nested = zip::ZipArchive::new(std::fs::File::open(&target)?)
            .map_err(|e| invalid(&e.to_string()))?;
        if nested.by_name("preset.json").is_err() {
            return Err(invalid("nested character collections are not supported"));
        }
        drop(nested);
        let id = import_file(&temporary.0, &target)?;
        match read(&temporary.0, &id)?.content {
            Content::Character { .. } => {
                if character.replace(id).is_some() {
                    return Err(invalid("duplicate character in collection"));
                }
            }
            Content::Voice { .. } => {
                if voice.replace(id).is_some() {
                    return Err(invalid("duplicate voice in collection"));
                }
            }
            Content::Visual { .. } => {
                if visual.replace(id).is_some() {
                    return Err(invalid("duplicate appearance in collection"));
                }
            }
            Content::CharacterPack { .. } => {
                return Err(invalid("import complete character preset directly"));
            }
        }
    }
    let character = character.ok_or_else(|| invalid("collection needs a character description"))?;
    let p = read(&temporary.0, &character)?;
    let mut prefs = Preferences::default();
    prefs.generation.character_description = p.content.description().unwrap().into();
    prefs.generation.voice_profile = voice.unwrap_or_default();
    prefs.desktop.visual_preset = visual.unwrap_or_default();
    let id = save_current_pack(&temporary.0, p.name, &prefs)?;
    let complete = read(&temporary.0, &id)?;
    import_prepared(root, &complete, Some(&folder(&temporary.0, &id)?))
}
