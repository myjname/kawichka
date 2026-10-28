use super::*;
use kavychka_core::preferences::Preferences;
fn fixture() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/character-pack-tests");
    std::fs::create_dir_all(&base).unwrap();
    let t = tempfile::tempdir_in(base).unwrap();
    std::fs::create_dir(t.path().join("presets")).unwrap();
    std::fs::create_dir(t.path().join("jobs")).unwrap();
    t
}
fn wav(value: i16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend(32036u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(32000u32.to_le_bytes());
    for _ in 0..16000 {
        b.extend(value.to_le_bytes());
    }
    b
}
fn components(root: &Path) -> (String, String, String) {
    let c = save_character(
        root,
        "",
        "Example character".into(),
        "Neutral reminder companion.".into(),
    )
    .unwrap();
    let media = root.join("prepared");
    std::fs::create_dir(&media).unwrap();
    std::fs::write(media.join("clip.wav"), wav(1)).unwrap();
    let v = import_prepared(
        root,
        &Preset {
            version: 1,
            name: "Example voice".into(),
            content: Content::Voice {
                references: vec![Reference {
                    file: "clip.wav".into(),
                    transcript: "A clean reference.".into(),
                }],
            },
        },
        Some(&media),
    )
    .unwrap();
    std::fs::write(media.join("pet.gif"),b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff\x21\xf9\x04\x01\0\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0\x3b").unwrap();
    let a = import_prepared(
        root,
        &Preset {
            version: 1,
            name: "Example appearance".into(),
            content: Content::Visual {
                file: "pet.gif".into(),
                layout: "pixel".into(),
            },
        },
        Some(&media),
    )
    .unwrap();
    (c, v, a)
}
#[test]
fn complete_character_is_self_contained_editable_and_roundtrips() {
    let t = fixture();
    let (c, v, a) = components(t.path());
    let mut prefs = Preferences::default();
    prefs.generation.character_description = "Custom description.".into();
    prefs.generation.voice_profile = v.clone();
    prefs.desktop.visual_preset = a.clone();
    prefs.generation.cosy_style = "confident".into();
    let id = save_current_pack(t.path(), "Complete example".into(), &prefs).unwrap();
    let mut p = read(t.path(), &id).unwrap();
    assert_eq!(p.content.description(), Some("Custom description."));
    assert_eq!(p.content.references().unwrap().len(), 1);
    assert_eq!(p.content.visual(), Some(("visual.gif", "pixel")));
    assert_eq!(
        std::fs::read(folder(t.path(), &id).unwrap().join("voice0.wav")).unwrap(),
        wav(1)
    );
    let raw = serde_json::to_value(&p).unwrap();
    assert_eq!(raw["delivery"], "custom");
    assert!(!raw["instruction"].as_str().unwrap().is_empty());
    assert!(raw.get("model_paths").is_none());
    let archive = t.path().join("complete.kavychka");
    export(t.path(), &id, &archive).unwrap();
    let copy = import_file(t.path(), &archive).unwrap();
    assert_eq!(
        serde_json::to_value(read(t.path(), &copy).unwrap()).unwrap(),
        raw
    );
    // Overrides remain independent: a selected pack does not replace explicit
    // voice/appearance fields merely because its character ID remains selected.
    prefs.desktop.character_preset = id.clone();
    prefs.generation.voice_profile = String::new();
    prefs.desktop.visual_preset = String::new();
    let overridden = save_current_pack(t.path(), "Overrides".into(), &prefs).unwrap();
    let override_preset = read(t.path(), &overridden).unwrap();
    assert!(override_preset.content.references().is_none());
    assert!(override_preset.content.visual().is_none());
    p.name = "Edited example".into();
    if let Content::CharacterPack { description, .. } = &mut p.content {
        *description = "Edited description.".into();
    }
    update(t.path(), &id, &p).unwrap();
    assert_eq!(read(t.path(), &id).unwrap().name, "Edited example");
    delete(t.path(), &id).unwrap();
    assert!(read(t.path(), &copy).is_ok());
    assert!(read(t.path(), &c).is_ok());
    assert!(read(t.path(), &v).is_ok());
    assert!(read(t.path(), &a).is_ok());
    assert_eq!(std::fs::read_dir(t.path().join("jobs")).unwrap().count(), 0);
}
#[test]
fn old_component_collection_imports_as_one_portable_character_atomically() {
    let t = fixture();
    let (c, v, a) = components(t.path());
    let zip = t.path().join("old-user-collection.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    for (i, id) in [c, v, a].iter().enumerate() {
        let single = t.path().join(format!("part{i}.kavychka"));
        export(t.path(), id, &single).unwrap();
        z.start_file(
            format!("part{i}.kavychka"),
            zip::write::FileOptions::default(),
        )
        .unwrap();
        z.write_all(&std::fs::read(single).unwrap()).unwrap();
    }
    z.start_file("README.md", zip::write::FileOptions::default())
        .unwrap();
    z.write_all(b"Instructions only; do not publish untyped text.")
        .unwrap();
    z.finish().unwrap();
    let before = list(t.path()).unwrap().len();
    let id = import_file(t.path(), &zip).unwrap();
    assert_eq!(list(t.path()).unwrap().len(), before + 1);
    let p = read(t.path(), &id).unwrap();
    assert!(matches!(p.content, Content::CharacterPack { .. }));
    assert_eq!(p.content.references().unwrap().len(), 1);
    assert!(p.content.visual().is_some());
    assert!(!folder(t.path(), &id).unwrap().join("README.md").exists());
    assert_eq!(std::fs::read_dir(t.path().join("jobs")).unwrap().count(), 0);
    let malformed = t.path().join("bad-collection.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&malformed).unwrap());
    z.start_file("part.kavychka", zip::write::FileOptions::default())
        .unwrap();
    z.write_all(&std::fs::read(t.path().join("part0.kavychka")).unwrap())
        .unwrap();
    z.start_file("broken.kavychka", zip::write::FileOptions::default())
        .unwrap();
    z.write_all(b"not a zip").unwrap();
    z.finish().unwrap();
    let count = list(t.path()).unwrap().len();
    assert!(import_file(t.path(), &malformed).is_err());
    assert_eq!(list(t.path()).unwrap().len(), count);
    assert_eq!(std::fs::read_dir(t.path().join("jobs")).unwrap().count(), 0);
}
#[test]
fn pack_media_and_delivery_reject_unsafe_or_unsupported_inputs() {
    let t = fixture();
    let p = Preset {
        version: 1,
        name: "Valid".into(),
        content: Content::CharacterPack {
            description: "Safe".into(),
            references: vec![],
            visual: None,
            delivery: "custom".into(),
            instruction: "".into(),
        },
    };
    assert!(p.validate(t.path()).is_err());
    let raw=br#"{"version":1,"name":"Safe","kind":"character_pack","description":"Description","references":[],"visual":null,"ApiKey":"dummy-secret"}"#;
    let p = parse(raw).unwrap();
    p.validate(t.path()).unwrap();
    assert!(!serde_json::to_string(&p).unwrap().contains("dummy-secret"));
    let mut p = p;
    if let Content::CharacterPack { references, .. } = &mut p.content {
        references.push(Reference {
            file: "../clip.wav".into(),
            transcript: String::new(),
        });
    }
    assert!(p.validate(t.path()).is_err());
}
