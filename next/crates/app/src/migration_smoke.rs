//! Synthetic native scenario: legacy semantics and independent preset operations.
use crate::{
    AppWindow,
    service::Backend,
    shell::{self, Shell},
};
use serde_json::json;
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
fn fixture(source: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(source.join("presets"))?;
    for id in ["voice01", "character01", "visual01"] {
        std::fs::create_dir(source.join("presets").join(id))?;
    }
    let now = chrono::Utc::now();
    let started = now - chrono::Duration::seconds(60);
    let next = now + chrono::Duration::minutes(4);
    let settings = json!({"SchemaVersion":2,"SetupComplete":true,"TimeZoneId":"Central Europe Standard Time","ThemeMode":"dark","UiLanguage":"en","SpeechLanguage":"en","VoiceProvider":"System","Voice":true,"NotificationSound":false,"PetSize":180,"PetX":100,"PetY":200,"CharacterFile":"character01","VisualFile":"visual01","VoiceProfile":"voice01","RunAtLogin":true,"AutomaticUpdates":false,"DeferGenerationFullscreen":false,"AutomaticPreparation":false,"BackendDailySettingsMigrated":true,"HandledEvents":[format!("{}:600:break",now.date_naive())],"ApiKey":"unit-secret"});
    std::fs::write(source.join("settings.json"), serde_json::to_vec(&settings)?)?;
    let checkpoint = json!({"Version":1,"Category":"break","Text":"Synthetic saved reminder","MessageUtc":started,"CheckpointUtc":now,"SuspendedUtc":null,"Pending":true,"Manual":false,"Followup":true,"Cycle":{"Category":"break","StartedUtc":started,"NextRepeatUtc":next,"RepeatsSent":1,"Finished":false,"HiddenByConfirmation":false}});
    std::fs::write(
        source.join("reminder-state.json"),
        serde_json::to_vec(&checkpoint)?,
    )?;
    for (id, p) in [
        (
            "character01",
            json!({"version":1,"kind":"character","name":"QA character","description":"An independent neutral character."}),
        ),
        (
            "visual01",
            json!({"version":1,"kind":"visual","name":"QA atlas","file":"atlas.png","layout":"codex"}),
        ),
        (
            "voice01",
            json!({"version":1,"kind":"voice","name":"QA voice","references":[{"file":"one.wav","transcript":"One reference."},{"file":"two.wav","transcript":"Second reference."}]}),
        ),
    ] {
        std::fs::write(
            source.join("presets").join(id).join("preset.json"),
            serde_json::to_vec(&p)?,
        )?;
    }
    let image = image::RgbaImage::from_fn(1536, 1872, |x, y| {
        image::Rgba([(x / 192 * 25) as u8, (y / 208 * 25) as u8, 130, 255])
    });
    image.save(source.join("presets/visual01/atlas.png"))?;
    for file in ["one.wav", "two.wav"] {
        let mut b = vec![];
        let size = 32000u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(size + 36).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&32000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&size.to_le_bytes());
        for i in 0..16000 {
            let v = ((i as f32 * 0.0864).sin() * 3000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(source.join("presets/voice01").join(file), b)?;
    }
    Ok(())
}
pub fn run(evidence: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !evidence.is_absolute() || evidence.exists() {
        return Err("use a new absolute evidence folder".into());
    }
    std::fs::create_dir_all(&evidence)?;
    let source = evidence.join("legacy-source");
    fixture(&source)?;
    let before = kavychka_storage::preview::inventory(&source, &Default::default())?.fingerprint;
    let data = source.join(".kavychka-next");
    crate::setup::prepare(&data, &source).map_err(|e| e.to_string())?;
    {
        let s = kavychka_storage::sandbox::Sandbox::open(&data)?;
        let p = s.load()?;
        let state = s.load_state()?;
        if p.desktop.time_zone != "Central Europe Standard Time"
            || p.desktop.pet_size != 180
            || !p.desktop.run_at_login
            || p.desktop.automatic_updates
            || p.desktop.defer_generation_fullscreen
            || p.desktop.visual_preset != "visual01"
            || p.desktop.character_preset != "character01"
            || p.generation.character_description != "An independent neutral character."
            || p.generation.voice_profile != "voice01"
            || state.cycles[0].sent != 1
            || state.pending[0].text != "Synthetic saved reminder"
            || !state.pending[0].visible
            || state.pet_position != Some((100, 200))
        {
            return Err("legacy settings/checkpoint semantics mismatch".into());
        }
        if serde_json::to_string(&p)?.contains("unit-secret") {
            return Err("credential imported".into());
        }
    }
    let archive = evidence.join("character.kavychka");
    kavychka_storage::presets::export(&source, "character01", &archive)?;
    shell::select_backend(false)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_runtime(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    let companion = Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let outcome = Rc::new(RefCell::new(None::<Result<(), String>>));
    let result = outcome.clone();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let own = companion.clone();
    let phase = Rc::new(Cell::new(0));
    let stage = phase.clone();
    let started = Instant::now();
    let output = evidence.clone();
    let mut imported = String::new();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let check = (|| -> Result<(), Box<dyn std::error::Error>> {
                if started.elapsed() > Duration::from_secs(120) {
                    return Err(format!("migration UI timed out at {}", stage.get()).into());
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string().into());
                }
                if !ui.get_ready() || ui.get_library_json().is_empty() {
                    return Ok(());
                }
                let view: crate::models_library::View =
                    serde_json::from_str(ui.get_library_json().as_str())?;
                match stage.get() {
                    0 if own.pet.get_custom_visual() => {
                        ui.set_page(4);
                        if ui.get_pet_size() != 180
                            || ui.get_time_zone() != "Central Europe Standard Time"
                            || view.presets.len() != 3
                        {
                            return Err("imported settings/presets not rendered".into());
                        }
                        let p = ui.window().take_snapshot()?;
                        image::save_buffer(
                            output.join("presets.png"),
                            p.as_bytes(),
                            p.width(),
                            p.height(),
                            image::ColorType::Rgba8,
                        )?;
                        ui.set_import_path(archive.to_string_lossy().as_ref().into());
                        ui.invoke_import_preset();
                        stage.set(1);
                    }
                    1 if view.presets.len() == 4 => {
                        imported = view
                            .presets
                            .iter()
                            .find(|e| {
                                e.id != "character01"
                                    && matches!(
                                        e.preset.content,
                                        kavychka_storage::presets::Content::Character { .. }
                                    )
                            })
                            .ok_or("new character absent")?
                            .id
                            .clone();
                        let ids = ui.get_preset_ids();
                        let index = (0..ids.row_count())
                            .find(|i| ids.row_data(*i).is_some_and(|s| s == imported.as_str()))
                            .ok_or("editor preset absent")?;
                        ui.set_preset_index(index as i32);
                        ui.invoke_preset_selected();
                        ui.set_preset_name("Edited QA character".into());
                        ui.set_preset_body("A different neutral character.".into());
                        ui.invoke_save_preset();
                        stage.set(2);
                    }
                    2 if view
                        .presets
                        .iter()
                        .any(|e| e.preset.name == "Edited QA character") =>
                    {
                        ui.invoke_export_preset();
                        stage.set(3);
                    }
                    3 if view.status.starts_with("Пресет / Preset:") => {
                        ui.invoke_delete_preset();
                        stage.set(4);
                    }
                    4 if view.presets.len() == 3 => {
                        if view.presets.iter().any(|e| e.id == imported) {
                            return Err("deleted preset still listed".into());
                        }
                        *result.borrow_mut() = Some(Ok(()));
                        ui.invoke_quit();
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(e) = check {
                *result.borrow_mut() = Some(Err(e.to_string()));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    drop(companion);
    drop(backend);
    outcome
        .borrow_mut()
        .take()
        .ok_or("UI ended without verification")?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let after = kavychka_storage::preview::inventory(&source, &Default::default())?.fingerprint;
    if before != after {
        return Err("legacy source changed".into());
    }
    {
        let s = kavychka_storage::sandbox::Sandbox::open(&data)?;
        let p = s.load()?;
        if p.desktop.character_preset != "character01"
            || p.generation.voice_profile != "voice01"
            || s.load_state()?.cycles[0].sent != 1
        {
            return Err("reopen lost selected presets or repeat budget".into());
        }
    }
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"real_windows":true,"legacy_settings_active":true,"checkpoint_and_repeat_budget_preserved":true,"named_timezone_active":true,"codex_atlas_rendered":true,"character_import_edit_export_delete":true,"independent_presets_preserved":true,"source_fingerprint_before":before,"source_fingerprint_after":after,"original_unchanged":true,"credentials_excluded":true,"reopened":true}),
        )?,
    )?;
    println!("Migration/preset UI passed: {}", evidence.display());
    Ok(())
}
