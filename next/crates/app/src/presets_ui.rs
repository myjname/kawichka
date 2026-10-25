use crate::{
    AppWindow,
    service::{Backend, Command},
};
use kavychka_core::preferences::Preferences;
use kavychka_storage::presets::{Content, Entry, Preset};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::path::PathBuf;
pub enum Action {
    Import(PathBuf, String, String),
    Save(String, Preset),
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
    let mut names = vec!["Новый характер / New character".into()];
    let mut chars = vec![String::new()];
    let mut charnames = vec!["Своё описание / Custom description".into()];
    let mut visuals = vec![String::new()];
    let mut visualnames = vec!["Кавычка / Kavychka".into()];
    for entry in entries {
        ids.push(entry.id.clone());
        names.push(format!(
            "{} · {}",
            entry.preset.name,
            match entry.preset.content {
                Content::Character { .. } => "Character",
                Content::Voice { .. } => "Voice",
                Content::Visual { .. } => "Visual",
            }
        ));
        match entry.preset.content {
            Content::Character { .. } => {
                chars.push(entry.id.clone());
                charnames.push(entry.preset.name.clone());
            }
            Content::Visual { .. } => {
                visuals.push(entry.id.clone());
                visualnames.push(entry.preset.name.clone());
            }
            _ => (),
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
                _ => "auto",
            }
            .into()
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
                _ => 0,
            });
        }
    }
    ui.set_preset_json(serde_json::to_string(&p).map_err(|e| e.to_string())?.into());
    Ok(())
}
pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    ui.on_character_selected(move || {
        if let Some(ui) = weak.upgrade() {
            let selected = id(ui.get_character_ids(), ui.get_character_index());
            if let Ok(entries) = entries(&ui)
                && let Some(Entry {
                    preset:
                        Preset {
                            content: Content::Character { description },
                            ..
                        },
                    ..
                }) = entries.into_iter().find(|e| e.id == selected)
            {
                ui.set_character_description(description.into());
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_choose_preset_file(move || {
        if let Some(ui) = weak.upgrade()
            && let Some(path) = kavychka_platform::dialogs::open()
        {
            ui.set_import_path(path.to_string_lossy().as_ref().into());
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
            if let Content::Voice { references } = p.content
                && let Some(r) = references.get(ui.get_reference_index().max(0) as usize)
            {
                ui.set_preset_body(r.transcript.as_str().into());
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
                        _ => Action::Delete(chosen),
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
