//! Real preset callbacks/service on a software window, never desktop input/audio.
use super::*;
use kavychka_storage::{
    presets::{self, Reference},
    sandbox::Sandbox,
};
use slint::platform::{
    EventLoopProxy, Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{
    collections::VecDeque,
    fs,
    io::Write,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
type Events = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;
struct Proxy(Events);
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}
struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    events: Events,
}
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.events.clone())))
    }
}
fn pump(events: &Events) {
    loop {
        let next = events.lock().unwrap().pop_front();
        if let Some(event) = next {
            event()
        } else {
            break;
        }
    }
    slint::platform::update_timers_and_animations();
}
fn wait(
    events: &Events,
    name: &str,
    condition: impl Fn() -> bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump(events);
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("Character callback timeout: {name}").into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn wave() -> Vec<u8> {
    let mut b = vec![];
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
    b.resize(32044, 0);
    b
}
fn loaded(ui: &AppWindow) -> Result<Preferences, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(ui.get_loaded_prefs().as_str())?)
}
/// The driver injects a failing SQLite trigger into this owned fixture before
/// the service starts. This exercises the real failed commit, not a UI mock.
pub fn rollback(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut original = store.load()?;
    original.desktop.setup_complete = true;
    original.desktop.automatic_updates = false;
    original.generation.auto_daily = false;
    original.generation.text_enabled = false;
    original.generation.tts_model = "system".into();
    store.save(&original)?;
    drop(store);
    let text = output.join("new-character.txt");
    fs::write(&text, "A concise synthetic new character description.")?;
    fs::write(
        output.join("ready-for-fault.json"),
        serde_json::to_vec(&data.join("shell.sqlite3"))?,
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !output.join("fault-installed").is_file() {
        if Instant::now() > deadline {
            return Err("Fault driver timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_audio_refreshing(true);
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    wait(&events, "initial preferences", || {
        ui.get_ready() && ui.get_character_ids().row_count() > 0
    })?;
    ui.set_user_name("Unsaved name".into());
    ui.set_start_time("invalid draft".into());
    ui.invoke_edited();
    import_character_path(&ui, &backend.sender, text)?;
    wait(&events, "failed import commit", || {
        ui.get_failed() && ui.get_status().contains("synthetic-import-save-fault")
    })?;
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        pump(&events);
        std::thread::sleep(Duration::from_millis(5));
    }
    if serde_json::to_value(loaded(&ui)?)? != serde_json::to_value(&original)?
        || !id(ui.get_character_ids(), ui.get_character_index()).is_empty()
        || ui.get_user_name() != "Unsaved name"
        || ui.get_start_time() != "invalid draft"
        || ui
            .get_character_feedback()
            .contains("импортирован и применён")
    {
        return Err("Failed import changed saved preferences or unrelated draft".into());
    }
    ui.hide()?;
    drop(backend);
    let stored = Sandbox::open(&data)?;
    if serde_json::to_value(stored.load()?)? != serde_json::to_value(&original)? {
        return Err("Failed commit persisted changed preferences".into());
    }
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"actual_service":true,"actual_sqlite_commit_failure":true,"saved_preferences_preserved":true,"unrelated_invalid_draft_preserved":true,"no_false_applied_selection":true}),
        )?,
    )?;
    Ok(())
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let collection = output.join("components.zip");
    let source = Sandbox::open(&output.join("source"))?;
    let character = presets::save_character(
        &source.root,
        "",
        "Synthetic complete character".into(),
        "A concise neutral reminder companion.".into(),
    )?;
    let media = output.join("media");
    fs::create_dir(&media)?;
    fs::write(media.join("voice.wav"), wave())?;
    image::RgbaImage::from_pixel(16, 16, image::Rgba([80, 140, 200, 255]))
        .save(media.join("pet.png"))?;
    let voice = presets::import_prepared(
        &source.root,
        &Preset {
            version: 1,
            name: "Synthetic voice".into(),
            content: Content::Voice {
                references: vec![Reference {
                    file: "voice.wav".into(),
                    transcript: "Prepared reference.".into(),
                }],
            },
        },
        Some(&media),
    )?;
    let visual = presets::import_prepared(
        &source.root,
        &Preset {
            version: 1,
            name: "Synthetic appearance".into(),
            content: Content::Visual {
                file: "pet.png".into(),
                layout: "pixel".into(),
            },
        },
        Some(&media),
    )?;
    let mut archive = zip::ZipWriter::new(fs::File::create(&collection)?);
    for (i, id) in [character, voice, visual].iter().enumerate() {
        let path = output.join(format!("component{i}.kavychka"));
        presets::export(&source.root, id, &path)?;
        archive.start_file(
            format!("component{i}.kavychka"),
            zip::write::FileOptions::default(),
        )?;
        archive.write_all(&fs::read(path)?)?;
    }
    archive.finish()?;
    drop(source);
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut original = store.load()?;
    original.desktop.setup_complete = true;
    original.generation.auto_daily = false;
    original.generation.text_enabled = false;
    original.generation.tts_model = "system".into();
    store.save(&original)?;
    drop(store);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    wait(&events, "initial preferences", || {
        ui.get_ready() && ui.get_character_ids().row_count() > 0
    })?;
    ui.set_page(4);
    // The native picker only supplies a path; use the very same submit boundary
    // as the main Import Character action, without opening a desktop dialog.
    ui.set_user_name("Unrelated unsaved draft".into());
    ui.invoke_edited();
    import_character_path(&ui, &backend.sender, collection)?;
    wait(
        &events,
        "collection import and automatic application",
        || {
            ui.get_dirty()
                && ui.get_character_index() > 0
                && ui.get_voice_profile_index() > 0
                && ui.get_visual_index() > 0
                && loaded(&ui).is_ok_and(|p| !p.desktop.character_preset.is_empty())
        },
    )?;
    let first = id(ui.get_character_ids(), ui.get_character_index());
    if first != id(ui.get_voice_profile_ids(), ui.get_voice_profile_index())
        || first != id(ui.get_visual_ids(), ui.get_visual_index())
        || !ui.get_preset_has_visual()
    {
        return Err("Combined selection did not apply all components".into());
    }
    let mut expected = original.clone();
    apply_to_preferences(
        &mut expected,
        &entries(&ui)?
            .into_iter()
            .find(|e| e.id == first)
            .ok_or("missing imported character")?,
    );
    if serde_json::to_value(loaded(&ui)?)? != serde_json::to_value(&expected)?
        || ui.get_user_name() != "Unrelated unsaved draft"
        || !ui.get_character_feedback().contains("импортирован")
    {
        return Err("Automatic import failed or changed unrelated saved/draft settings".into());
    }
    ui.set_user_name(original.user_name.as_str().into());
    ui.invoke_save();
    wait(&events, "explicit save", || {
        !ui.get_dirty() && loaded(&ui).is_ok_and(|p| p.desktop.character_preset == first)
    })?;
    ui.set_voice_profile_index(0);
    ui.set_visual_index(0);
    ui.set_character_description("Independently customized description.".into());
    ui.set_delivery_index(1);
    ui.set_custom_instruction("Speak calmly and clearly.".into());
    ui.invoke_edited();
    ui.invoke_save();
    wait(&events, "independent overrides", || {
        !ui.get_dirty()
            && loaded(&ui).is_ok_and(|p| {
                p.desktop.character_preset == first
                    && p.generation.voice_profile.is_empty()
                    && p.desktop.visual_preset.is_empty()
                    && p.generation.cosy_style == "custom"
                    && p.generation.character_description == "Independently customized description."
            })
    })?;
    ui.set_character_pack_name("Snapshot with overrides".into());
    let saved_start = ui.get_start_time();
    ui.set_start_time("invalid draft".into());
    ui.invoke_edited();
    ui.invoke_save_character_pack();
    wait(&events, "save current as complete character", || {
        ui.get_dirty() && id(ui.get_character_ids(), ui.get_character_index()) != first
    })?;
    let second = id(ui.get_character_ids(), ui.get_character_index());
    if ui.get_start_time() != "invalid draft" {
        return Err("Preset operation replaced unrelated invalid schedule draft".into());
    }
    ui.set_start_time(saved_start);
    if !id(ui.get_voice_profile_ids(), ui.get_voice_profile_index()).is_empty()
        || !id(ui.get_visual_ids(), ui.get_visual_index()).is_empty()
        || ui.get_delivery_index() != 1
        || ui.get_character_description() != "Independently customized description."
        || loaded(&ui)?.generation.character_description != "Independently customized description."
        || ui.get_preset_has_visual()
    {
        return Err("Saved character lost independent overrides".into());
    }
    ui.set_preset_index(choose(ui.get_preset_ids(), &second));
    ui.invoke_preset_selected();
    ui.set_library_status("".into());
    ui.invoke_delete_preset();
    if !(ui.get_library_status().contains("Сначала выбери")
        || ui.get_library_status().contains("Select another"))
        || !ui.get_preset_ids().iter().any(|v| v == second)
        || loaded(&ui)?.desktop.character_preset != first
    {
        return Err("Deleting a character selected in an unsaved draft was not blocked".into());
    }
    ui.invoke_save();
    wait(&events, "save snapshot selection", || {
        !ui.get_dirty() && loaded(&ui).is_ok_and(|p| p.desktop.character_preset == second)
    })?;
    ui.set_preset_index(choose(ui.get_preset_ids(), &first));
    ui.invoke_preset_selected();
    ui.set_preset_name("Edited complete character".into());
    ui.set_preset_body("Edited packaged description.".into());
    ui.invoke_save_preset();
    wait(&events, "edit inactive complete character", || {
        entries(&ui).is_ok_and(|items| {
            items.iter().any(|entry| {
                entry.id == first
                    && entry.preset.name == "Edited complete character"
                    && entry.preset.content.description() == Some("Edited packaged description.")
            })
        })
    })?;
    if loaded(&ui)?.generation.character_description != "Independently customized description." {
        return Err("Editing another character overwrote current character customization".into());
    }
    ui.set_character_index(choose(ui.get_character_ids(), &first));
    ui.invoke_character_selected();
    ui.invoke_edited();
    if id(ui.get_voice_profile_ids(), ui.get_voice_profile_index()) != first
        || id(ui.get_visual_ids(), ui.get_visual_index()) != first
        || ui.get_delivery_index() != 0
        || ui.get_character_description() != "Edited packaged description."
    {
        return Err("Reselecting character did not restore its bundled components".into());
    }
    ui.invoke_save();
    wait(&events, "reselect original", || {
        !ui.get_dirty()
            && loaded(&ui).is_ok_and(|p| {
                p.desktop.character_preset == first
                    && p.generation.voice_profile == first
                    && p.desktop.visual_preset == first
            })
    })?;
    ui.set_preset_index(choose(ui.get_preset_ids(), &first));
    ui.invoke_preset_selected();
    ui.invoke_delete_preset();
    wait(&events, "active delete protection", || {
        ui.get_library_status().contains("Сначала выбери")
            || ui.get_library_status().contains("Select another")
    })?;
    ui.set_preset_index(choose(ui.get_preset_ids(), &second));
    ui.invoke_preset_selected();
    ui.invoke_delete_preset();
    wait(&events, "inactive character deletion", || {
        !ui.get_preset_ids().iter().any(|v| v == second)
    })?;
    ui.set_preset_index(choose(ui.get_preset_ids(), &first));
    ui.invoke_preset_selected();
    ui.invoke_export_preset();
    wait(&events, "export character", || {
        fs::read_dir(data.join("exports")).is_ok_and(|items| {
            items
                .filter_map(Result::ok)
                .any(|e| e.path().extension().is_some_and(|s| s == "kavychka"))
        })
    })?;
    ui.set_preset_index(0);
    ui.invoke_preset_selected();
    if ui.get_preset_has_visual() || ui.get_reference_names().row_count() != 1 {
        return Err("New description editor retained visual/reference controls".into());
    }
    window.request_redraw();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 780);
    window.draw_if_needed(|r| {
        r.render(pixels.make_mut_slice(), 1100);
    });
    image::save_buffer(
        output.join("character.png"),
        pixels.as_bytes(),
        1100,
        780,
        image::ColorType::Rgb8,
    )?;
    ui.hide()?;
    drop(backend);
    let stored = Sandbox::open(&data)?;
    let final_prefs = stored.load()?;
    if final_prefs.desktop.character_preset != first
        || final_prefs.generation.voice_profile != first
        || final_prefs.desktop.visual_preset != first
    {
        return Err("Final persisted selection changed".into());
    }
    if presets::read(&data, &first)?.content.references().is_none()
        || presets::read(&data, &second).is_ok()
    {
        return Err("Character deletion removed wrong media".into());
    }
    let proof = serde_json::json!({"passed":true,"native_windows_created":false,"audio_played":false,"models_used":false,"actual_ui_callbacks":true,"actual_service":true,"one_archive_import_applied_and_saved_character":true,"unrelated_unsaved_draft_preserved":true,"independent_overrides_preserved":true,"save_current_snapshot":true,"edit_complete_character":true,"unrelated_preset_actions_preserve_description_override":true,"reselect_restores_components":true,"active_delete_protected":true,"unsaved_draft_delete_protected":true,"inactive_delete_preserved_active":true,"exported":true});
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&proof)?,
    )?;
    println!("{proof}");
    Ok(())
}
