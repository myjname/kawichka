use crate::{
    AppWindow,
    service::{Backend, Command},
};
use kavychka_core::preferences::Preferences;
use kavychka_storage::presets::{Content, Entry, Preset};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
mod acceptance;
pub fn verify_complete_character(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    acceptance::run(output)
}
pub enum Action {
    Import(PathBuf, String, String),
    Save(String, Preset),
    SavePack(String, Box<Preferences>),
    Export(String),
    Delete(String),
}
fn strings(v: Vec<String>) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        v.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}
fn id(ids: ModelRc<slint::SharedString>, index: i32) -> String {
    ids.row_data(index.max(0) as usize)
        .map(|s| s.to_string())
        .unwrap_or_default()
}
pub fn render(ui: &AppWindow, entries: &[Entry]) {
    let selected = id(ui.get_preset_ids(), ui.get_preset_index());
    let loaded = serde_json::from_str::<Preferences>(ui.get_loaded_prefs().as_str()).ok();
    let character = if ui.get_character_ids().row_count() == 0 {
        loaded
            .as_ref()
            .map(|p| p.desktop.character_preset.clone())
            .unwrap_or_default()
    } else {
        id(ui.get_character_ids(), ui.get_character_index())
    };
    let visual = if ui.get_visual_ids().row_count() == 0 {
        loaded
            .as_ref()
            .map(|p| p.desktop.visual_preset.clone())
            .unwrap_or_default()
    } else {
        id(ui.get_visual_ids(), ui.get_visual_index())
    };
    let mut ids = vec![String::new()];
    let en = ui.get_english();
    let mut names = vec![
        if en {
            "New character"
        } else {
            "Новый персонаж"
        }
        .into(),
    ];
    let mut chars = vec![String::new()];
    let mut charnames = vec![
        if en {
            "Custom components"
        } else {
            "Свои компоненты"
        }
        .into(),
    ];
    let mut visuals = vec![String::new()];
    let mut visualnames = vec![if en { "Kavychka" } else { "Кавычка" }.into()];
    for entry in entries {
        ids.push(entry.id.clone());
        names.push(format!(
            "{} · {}",
            entry.preset.name,
            match entry.preset.content {
                Content::Character { .. } =>
                    if en {
                        "character"
                    } else {
                        "характер"
                    },
                Content::Voice { .. } =>
                    if en {
                        "voice"
                    } else {
                        "голос"
                    },
                Content::Visual { .. } =>
                    if en {
                        "appearance"
                    } else {
                        "внешность"
                    },
                Content::CharacterPack { .. } =>
                    if en {
                        "complete character"
                    } else {
                        "персонаж целиком"
                    },
            }
        ));
        if entry.preset.content.description().is_some() {
            chars.push(entry.id.clone());
            charnames.push(
                if matches!(entry.preset.content, Content::Character { .. }) {
                    format!(
                        "{} · {}",
                        entry.preset.name,
                        if en {
                            "character only"
                        } else {
                            "только характер"
                        }
                    )
                } else {
                    entry.preset.name.clone()
                },
            );
        }
        if entry.preset.content.visual().is_some() {
            visuals.push(entry.id.clone());
            visualnames.push(entry.preset.name.clone());
        }
    }
    ui.set_preset_index(ids.iter().position(|i| *i == selected).unwrap_or(0) as i32);
    ui.set_preset_ids(strings(ids));
    ui.set_preset_names(strings(names));
    ui.set_character_ids(strings(chars));
    ui.set_character_names(strings(charnames));
    ui.set_visual_ids(strings(visuals));
    ui.set_visual_names(strings(visualnames));
    ui.set_character_index(
        ui.get_character_ids()
            .iter()
            .position(|v| v == character.as_str())
            .unwrap_or(0) as i32,
    );
    ui.set_visual_index(
        ui.get_visual_ids()
            .iter()
            .position(|v| v == visual.as_str())
            .unwrap_or(0) as i32,
    );
    // Only explicit preset operations produce this result. Restoring a profile
    // or refreshing a library must never silently replace the current draft.
    if let Ok(view) =
        serde_json::from_str::<crate::models_library::View>(ui.get_library_json().as_str())
        && !view.imported_preset.is_empty()
        && ui.get_last_imported_preset() != view.imported_preset
        && let Some(entry) = entries.iter().find(|e| e.id == view.imported_preset)
    {
        ui.set_last_imported_preset(entry.id.as_str().into());
        ui.set_preset_index(
            ui.get_preset_ids()
                .iter()
                .position(|s| s == entry.id.as_str())
                .unwrap_or(0) as i32,
        );
        apply_selection(ui, entry);
        let _ = select(ui);
        ui.invoke_edited();
    }
}
fn choose(ids: ModelRc<slint::SharedString>, value: &str) -> i32 {
    ids.iter().position(|s| s == value).unwrap_or(0) as i32
}
fn apply_selection(ui: &AppWindow, entry: &Entry) {
    match &entry.preset.content {
        Content::CharacterPack {
            description,
            references,
            visual,
            delivery,
            instruction,
        } => {
            ui.set_character_description(description.as_str().into());
            ui.set_character_index(choose(ui.get_character_ids(), &entry.id));
            ui.set_voice_profile_index(choose(
                ui.get_voice_profile_ids(),
                if references.is_empty() { "" } else { &entry.id },
            ));
            ui.set_visual_index(choose(
                ui.get_visual_ids(),
                if visual.is_none() { "" } else { &entry.id },
            ));
            ui.set_delivery_index(if delivery == "custom" { 1 } else { 0 });
            ui.set_custom_instruction(instruction.as_str().into());
        }
        Content::Character { description } => {
            ui.set_character_description(description.as_str().into());
            ui.set_character_index(choose(ui.get_character_ids(), &entry.id));
        }
        Content::Voice { .. } => {
            ui.set_voice_profile_index(choose(ui.get_voice_profile_ids(), &entry.id))
        }
        Content::Visual { .. } => ui.set_visual_index(choose(ui.get_visual_ids(), &entry.id)),
    }
}
pub fn apply(ui: &AppWindow, p: &Preferences) {
    if ui.get_time_zone_ids().row_count() == 0 {
        let zones = kavychka_platform::time::zones();
        ui.set_time_zone_ids(strings(zones.iter().map(|(id, _)| id.clone()).collect()));
        ui.set_time_zone_names(strings(zones.into_iter().map(|(_, name)| name).collect()));
    }
    let zones = ui.get_time_zone_ids();
    ui.set_time_zone_index(
        (0..zones.row_count())
            .find(|i| {
                zones
                    .row_data(*i)
                    .is_some_and(|id| id == p.desktop.time_zone.as_str())
            })
            .unwrap_or(0) as i32,
    );
    ui.set_time_zone(p.desktop.time_zone.as_str().into());
    ui.set_pet_size(p.desktop.pet_size.into());
    ui.set_run_at_login(p.desktop.run_at_login);
    ui.set_automatic_updates(p.desktop.automatic_updates);
    ui.set_generation_fullscreen(p.desktop.defer_generation_fullscreen);
    ui.set_recovery_limit_mb((p.desktop.recovery_limit_bytes / 1_000_000) as i32);
    for (ids, chosen, is_char) in [
        (ui.get_character_ids(), &p.desktop.character_preset, true),
        (ui.get_visual_ids(), &p.desktop.visual_preset, false),
    ] {
        let index = (0..ids.row_count())
            .find(|i| ids.row_data(*i).is_some_and(|s| s == chosen.as_str()))
            .unwrap_or(0) as i32;
        if is_char {
            ui.set_character_index(index)
        } else {
            ui.set_visual_index(index)
        }
    }
}
pub fn collect(ui: &AppWindow, p: &mut Preferences) -> Result<(), String> {
    p.desktop.time_zone = ui.get_time_zone().to_string().trim().into();
    p.desktop.pet_size = u16::try_from(ui.get_pet_size()).map_err(|e| e.to_string())?;
    p.desktop.run_at_login = ui.get_run_at_login();
    p.desktop.automatic_updates = ui.get_automatic_updates();
    p.desktop.defer_generation_fullscreen = ui.get_generation_fullscreen();
    p.desktop.recovery_limit_bytes =
        u64::try_from(ui.get_recovery_limit_mb()).map_err(|e| e.to_string())? * 1_000_000;
    if ui.get_character_ids().row_count() > 0 {
        p.desktop.character_preset = id(ui.get_character_ids(), ui.get_character_index());
    }
    if ui.get_visual_ids().row_count() > 0 {
        p.desktop.visual_preset = id(ui.get_visual_ids(), ui.get_visual_index());
    }
    p.desktop.validate()
}
fn entries(ui: &AppWindow) -> Result<Vec<Entry>, String> {
    serde_json::from_str::<crate::models_library::View>(ui.get_library_json().as_str())
        .map(|v| v.presets)
        .map_err(|e| e.to_string())
}
fn stash(ui: &AppWindow) -> Result<Preset, String> {
    let mut p: Preset = serde_json::from_str(ui.get_preset_json().as_str()).unwrap_or(Preset {
        version: 1,
        name: String::new(),
        content: Content::Character {
            description: String::new(),
        },
    });
    p.name = ui.get_preset_name().to_string();
    match &mut p.content {
        Content::Character { description } => *description = ui.get_preset_body().to_string(),
        Content::Voice { references } => {
            if let Some(r) = references.get_mut(ui.get_editing_reference().max(0) as usize) {
                r.transcript = ui.get_preset_body().to_string();
            }
        }
        Content::Visual { layout, .. } => {
            *layout = match ui.get_visual_layout() {
                1 => "image",
                2 => "codex",
                3 => "pixel",
                4 => "smooth",
                _ => "auto",
            }
            .into()
        }
        Content::CharacterPack {
            description,
            references,
            visual,
            ..
        } => {
            let n = ui.get_editing_reference().max(0) as usize;
            if n == 0 {
                *description = ui.get_preset_body().to_string();
            } else if let Some(r) = references.get_mut(n - 1) {
                r.transcript = ui.get_preset_body().to_string();
            }
            if let Some(v) = visual {
                v.layout = match ui.get_visual_layout() {
                    1 => "image",
                    2 => "codex",
                    3 => "pixel",
                    4 => "smooth",
                    _ => "auto",
                }
                .into();
            }
        }
    }
    ui.set_preset_json(serde_json::to_string(&p).map_err(|e| e.to_string())?.into());
    Ok(p)
}
fn select(ui: &AppWindow) -> Result<(), String> {
    let chosen = id(ui.get_preset_ids(), ui.get_preset_index());
    let p = entries(ui)?
        .into_iter()
        .find(|e| e.id == chosen)
        .map(|e| e.preset)
        .unwrap_or(Preset {
            version: 1,
            name: String::new(),
            content: Content::Character {
                description: String::new(),
            },
        });
    ui.set_preset_has_visual(p.content.visual().is_some());
    ui.set_preset_name(p.name.as_str().into());
    ui.set_reference_index(0);
    ui.set_editing_reference(0);
    ui.set_visual_layout(0);
    match &p.content {
        Content::Character { description } => {
            ui.set_preset_body(description.as_str().into());
            ui.set_reference_names(strings(vec!["Описание / Description".into()]));
        }
        Content::Voice { references } => {
            ui.set_reference_names(strings(references.iter().map(|r| r.file.clone()).collect()));
            ui.set_preset_body(references[0].transcript.as_str().into());
        }
        Content::Visual { file, layout } => {
            ui.set_reference_names(strings(vec![file.clone()]));
            ui.set_preset_body("".into());
            ui.set_visual_layout(match layout.as_str() {
                "image" => 1,
                "codex" => 2,
                "pixel" => 3,
                "smooth" => 4,
                _ => 0,
            });
        }
        Content::CharacterPack {
            description,
            references,
            visual,
            ..
        } => {
            let mut names = vec![
                if ui.get_english() {
                    "Character description"
                } else {
                    "Описание характера"
                }
                .into(),
            ];
            names.extend(references.iter().enumerate().map(|(i, _)| {
                format!(
                    "{} {}",
                    if ui.get_english() {
                        "Voice reference"
                    } else {
                        "Референс голоса"
                    },
                    i + 1
                )
            }));
            if let Some(v) = visual {
                names.push(
                    if ui.get_english() {
                        "Appearance"
                    } else {
                        "Внешность"
                    }
                    .into(),
                );
                ui.set_visual_layout(match v.layout.as_str() {
                    "image" => 1,
                    "codex" => 2,
                    "pixel" => 3,
                    "smooth" => 4,
                    _ => 0,
                });
            }
            ui.set_reference_names(strings(names));
            ui.set_preset_body(description.as_str().into());
        }
    }
    ui.set_preset_json(serde_json::to_string(&p).map_err(|e| e.to_string())?.into());
    Ok(())
}
fn import_character_path(
    ui: &AppWindow,
    sender: &SyncSender<Command>,
    path: PathBuf,
) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "kavychka" | "zip" | "json" | "txt") {
        return Err(if ui.get_english() {
            "Choose a character preset (.kavychka, .zip or .json) or a description (.txt)"
        } else {
            "Выбери пресет персонажа (.kavychka, .zip или .json) либо описание (.txt)"
        }
        .into());
    }
    ui.set_library_status(
        if ui.get_english() {
            "Checking character preset…"
        } else {
            "Проверка персонажа…"
        }
        .into(),
    );
    sender
        .try_send(Command::Preset(Box::new(Action::Import(
            path,
            "character".into(),
            String::new(),
        ))))
        .map_err(|e| e.to_string())
}
pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_import_character(move || {
        if let Some(ui) = weak.upgrade()
            && let Some(path) = kavychka_platform::dialogs::open()
            && let Err(error) = import_character_path(&ui, &sender, path)
        {
            ui.set_library_status(error.into());
        }
    });
    let weak = ui.as_weak();
    ui.on_character_selected(move || {
        if let Some(ui) = weak.upgrade() {
            let selected = id(ui.get_character_ids(), ui.get_character_index());
            if let Ok(entries) = entries(&ui)
                && let Some(entry) = entries.into_iter().find(|e| e.id == selected)
            {
                apply_selection(&ui, &entry);
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_choose_preset_file(move || {
        if let Some(ui) = weak.upgrade()
            && let Some(path) = kavychka_platform::dialogs::open()
        {
            ui.set_import_path(crate::display_path::display(&path).into());
        }
    });
    let weak = ui.as_weak();
    ui.on_preset_selected(move || {
        if let Some(ui) = weak.upgrade()
            && let Err(e) = select(&ui)
        {
            ui.set_library_status(e.into());
        }
    });
    let weak = ui.as_weak();
    ui.on_reference_selected(move || {
        if let Some(ui) = weak.upgrade()
            && let Ok(p) = stash(&ui)
        {
            ui.set_editing_reference(ui.get_reference_index());
            if let Content::Voice { references } = &p.content
                && let Some(r) = references.get(ui.get_reference_index().max(0) as usize)
            {
                ui.set_preset_body(r.transcript.as_str().into());
            } else if let Content::CharacterPack {
                description,
                references,
                ..
            } = &p.content
            {
                ui.set_preset_body(
                    if ui.get_reference_index() == 0 {
                        description.as_str()
                    } else {
                        references
                            .get(ui.get_reference_index().max(1) as usize - 1)
                            .map(|r| r.transcript.as_str())
                            .unwrap_or("")
                    }
                    .into(),
                );
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_save_character_pack(move || {
        if let Some(ui) = weak.upgrade() {
            let result = (|| {
                let prefs = crate::collect(&ui)?;
                let name = ui.get_character_pack_name().to_string();
                if name.trim().is_empty() {
                    return Err(if ui.get_english() {
                        "Enter a character name"
                    } else {
                        "Введи название персонажа"
                    }
                    .to_string());
                }
                sender
                    .try_send(Command::Preset(Box::new(Action::SavePack(
                        name,
                        Box::new(prefs),
                    ))))
                    .map_err(|e| e.to_string())
            })();
            if let Err(e) = result {
                ui.set_library_status(e.into());
            }
        }
    });
    for operation in 0..4 {
        let weak = ui.as_weak();
        let sender = backend.sender.clone();
        let action = move || {
            if let Some(ui) = weak.upgrade() {
                let result = (|| {
                    let chosen = id(ui.get_preset_ids(), ui.get_preset_index());
                    let action = match operation {
                        0 => Action::Import(
                            PathBuf::from(ui.get_import_path().as_str()),
                            match ui.get_import_kind() {
                                1 => "voice",
                                2 => "visual",
                                _ => "character",
                            }
                            .into(),
                            ui.get_import_transcript().to_string(),
                        ),
                        1 => Action::Save(chosen, stash(&ui)?),
                        2 => Action::Export(chosen),
                        _ => {
                            let draft = crate::collect(&ui)?;
                            if chosen == draft.desktop.character_preset
                                || chosen == draft.generation.voice_profile
                                || chosen == draft.desktop.visual_preset
                            {
                                return Err(if ui.get_english() {
                                    "Select another character, voice and appearance, then save settings before deleting this preset"
                                } else {
                                    "Сначала выбери другой характер, голос и внешность, затем сохрани настройки перед удалением пресета"
                                }.to_string());
                            }
                            Action::Delete(chosen)
                        }
                    };
                    sender
                        .try_send(Command::Preset(Box::new(action)))
                        .map_err(|e| e.to_string())
                })();
                if let Err(e) = result {
                    ui.set_library_status(e.into());
                }
            }
        };
        match operation {
            0 => ui.on_import_preset(action),
            1 => ui.on_save_preset(action),
            2 => ui.on_export_preset(action),
            _ => ui.on_delete_preset(action),
        }
    }
}
