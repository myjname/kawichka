//! Real Slint pointer/keyboard regression checks in an isolated software window.
//! Never creates OS windows, sends desktop input, enumerates devices or plays audio.
use crate::{AppWindow, Command, service::Backend};
use kavychka_core::preferences::{AnimationIntensity, Language};
use kavychka_storage::sandbox::Sandbox;
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, SharedString, VecModel,
    platform::{
        EventLoopProxy, Platform, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
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
        let e = events.lock().unwrap().pop_front();
        if let Some(e) = e { e() } else { break }
    }
    slint::platform::update_timers_and_animations();
}
fn wait(
    events: &Events,
    name: &str,
    condition: impl Fn() -> bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        pump(events);
        if condition() {
            return Ok(());
        }
        if Instant::now() > end {
            return Err(format!("Compact controls timeout: {name}").into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(ui: &AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        ui.window().dispatch_event(event);
    }
}
fn key(ui: &AppWindow, key: impl Into<SharedString>) {
    let text = key.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
fn next_choice(ui: &AppWindow, x: f32, y: f32) {
    click(ui, x, y);
    key(ui, slint::platform::Key::DownArrow);
    key(ui, "\n");
}
fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .iter()
            .map(|s| SharedString::from(*s))
            .collect::<Vec<_>>(),
    ))
}
fn render(window: &MinimalSoftwareWindow, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    let size = window.size();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size.width, size.height);
    window.draw_if_needed(|r| {
        r.render(pixels.make_mut_slice(), size.width as usize);
    });
    image::save_buffer(
        path,
        pixels.as_bytes(),
        size.width,
        size.height,
        image::ColorType::Rgb8,
    )?;
    Ok(())
}
fn focused_states(
    ui: &AppWindow,
    window: &MinimalSoftwareWindow,
    output: &Path,
    visual_pack: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let folder = output.join("focused");
    fs::create_dir(&folder)?;
    let mut captured = Vec::new();
    for (width, height) in [(720, 540), (1100, 780)] {
        window.set_size(slint::PhysicalSize::new(width, height));
        for (theme, label) in [(1, "light"), (2, "dark")] {
            ui.set_theme_index(theme);
            ui.set_page(0);
            ui.set_generation_running(true);
            ui.set_task_title("Model setup".into());
            ui.set_library_status("Preparing selected components".into());
            for (progress, state) in [(-1., "working"), (0.4, "40-percent")] {
                ui.set_task_progress(progress);
                let name = format!("task-{state}-{width}x{height}-{label}.png");
                render(window, &folder.join(&name))?;
                captured.push(name);
            }
            ui.set_generation_running(false);
            ui.set_page(4);
            ui.set_character_options(false);
            ui.set_preset_management(true);
            ui.set_preset_index(0);
            ui.invoke_preset_selected();
            if ui.get_preset_has_visual() || ui.get_reference_names().row_count() != 1 {
                return Err("New character editor exposed media controls".into());
            }
            ui.set_scroll_y(-ui.get_scroll_overflow());
            let name = format!("editor-new-{width}x{height}-{label}.png");
            render(window, &folder.join(&name))?;
            captured.push(name);
            let index = ui
                .get_preset_ids()
                .iter()
                .position(|id| id == visual_pack)
                .ok_or("Visual fixture missing from preset editor")?;
            ui.set_preset_index(index as i32);
            ui.invoke_preset_selected();
            if !ui.get_preset_has_visual()
                || ui.get_reference_names().row_count() != 2
                || ui.get_visual_layout() != 3
            {
                return Err("Visual character editor lost appearance/reference controls".into());
            }
            ui.set_scroll_y(-ui.get_scroll_overflow());
            let name = format!("editor-visual-{width}x{height}-{label}.png");
            render(window, &folder.join(&name))?;
            captured.push(name);
            click(ui, 600., height as f32 - 136.);
            let name = format!("editor-visual-options-{width}x{height}-{label}.png");
            render(window, &folder.join(&name))?;
            captured.push(name);
            key(ui, slint::platform::Key::Escape);
        }
    }
    Ok(captured)
}
fn prefs(
    ui: &AppWindow,
) -> Result<kavychka_core::preferences::Preferences, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(ui.get_loaded_prefs().as_str())?)
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut original = store.load()?;
    original.ui_language = Language::En;
    original.desktop.setup_complete = true;
    original.desktop.automatic_updates = false;
    original.reminders.animation = false;
    original.reminders.animation_intensity = AnimationIntensity::Expressive;
    original.schedule.break_every = 720;
    original.schedule.exercise_after_minutes = 1440;
    original.idle_minutes = 120;
    original.generation.auto_daily = false;
    original.generation.text_enabled = false;
    original.generation.tts_model = "system".into();
    store.save(&original)?;
    let media = output.join("visual-fixture");
    fs::create_dir(&media)?;
    image::RgbaImage::from_pixel(16, 16, image::Rgba([70, 130, 200, 255]))
        .save(media.join("pet.png"))?;
    let visual_pack = kavychka_storage::presets::import_prepared(
        &data,
        &kavychka_storage::presets::Preset {
            version: 1,
            name: "Visual editor fixture".into(),
            content: kavychka_storage::presets::Content::CharacterPack {
                description: "A neutral synthetic visual character.".into(),
                references: vec![],
                visual: Some(kavychka_storage::presets::PackVisual {
                    file: "pet.png".into(),
                    layout: "pixel".into(),
                }),
                delivery: "auto".into(),
                instruction: String::new(),
            },
        },
        Some(&media),
    )?;
    drop(store);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    // Suppress the production refresh callback's initial device enumeration.
    ui.set_audio_refreshing(true);
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.set_audio_refreshing(false);
    let pauses = Arc::new(Mutex::new(Vec::new()));
    let calls = pauses.clone();
    let sender = backend.sender.clone();
    ui.on_pause_minutes(move |minutes| {
        calls.lock().unwrap().push(minutes);
        let _ = sender.try_send(Command::Pause(minutes));
    });
    let audio_calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let calls = audio_calls.clone();
    let sender = backend.sender.clone();
    ui.on_silence(move || {
        calls.lock().unwrap().push("silence".into());
        let _ = sender.try_send(Command::Silence);
    });
    let calls = audio_calls.clone();
    ui.on_test_audio(move |kind| calls.lock().unwrap().push(format!("test_audio:{kind}")));
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    wait(&events, "initial view", || {
        ui.get_ready() && ui.get_preset_ids().row_count() > 1
    })?;
    if ui.get_break_minutes() != 720
        || ui.get_stretch_minutes() != 1440
        || ui.get_idle_minutes() != 120
    {
        return Err(
            "Previously supported schedule/activity bounds were changed while loading controls"
                .into(),
        );
    }
    ui.set_page(0);
    ui.set_next_reminder("Controlled reminder".into());
    ui.set_pending_text("Controlled single-line reminder".into());
    render(&window, &output.join("today-before.png"))?;
    click(&ui, 496., 437.);
    wait(&events, "Until tomorrow pointer action", || {
        pauses.lock().unwrap().len() == 1
    })?;
    if pauses.lock().unwrap()[0] != -1 {
        return Err("Until tomorrow did not dispatch Pause(-1)".into());
    }
    ui.set_pending_text("Controlled single-line reminder".into());
    click(&ui, 592., 437.);
    wait(&events, "Resume pointer action", || {
        pauses.lock().unwrap().len() == 2
    })?;
    if pauses.lock().unwrap()[1] != 0 {
        return Err("Resume did not dispatch Pause(0)".into());
    }
    ui.set_page(5);
    ui.set_time_zone_ids(strings(&["", "UTC"]));
    ui.set_time_zone_names(strings(&["Follow system", "UTC fixture"]));
    ui.set_time_zone_index(0);
    ui.set_time_zone("".into());
    render(&window, &output.join("settings-top-before.png"))?;
    next_choice(&ui, 600., 270.);
    pump(&events);
    if ui.get_time_zone_index() != 1 || ui.get_time_zone() != "UTC" {
        return Err(format!(
            "Timezone selection did not update stable ID: index={} value={}",
            ui.get_time_zone_index(),
            ui.get_time_zone()
        )
        .into());
    }
    ui.set_audio_device_ids(strings(&["", "synthetic-pinned-endpoint"]));
    ui.set_audio_device_names(strings(&["System fixture", "Pinned fixture"]));
    ui.set_audio_device_index(0);
    ui.set_audio_device_id("".into());
    ui.set_scroll_y(-ui.get_scroll_overflow());
    render(&window, &output.join("settings-bottom-before.png"))?;
    next_choice(&ui, 600., 377.);
    pump(&events);
    if ui.get_audio_device_index() != 1 || ui.get_audio_device_id() != "synthetic-pinned-endpoint" {
        return Err("Audio selection did not update stable endpoint ID".into());
    }
    click(&ui, 490., 418.);
    pump(&events);
    if *audio_calls.lock().unwrap() != ["silence"] {
        return Err(format!(
            "Stop button dispatched {:?} instead of silence",
            audio_calls.lock().unwrap()
        )
        .into());
    }
    next_choice(&ui, 600., 495.);
    pump(&events);
    if !ui.get_animation() || ui.get_animation_index() != 1 {
        return Err("Animation choice did not re-enable a previously disabled companion".into());
    }
    render(&window, &output.join("settings-edited.png"))?;
    click(&ui, 1000., 746.);
    wait(&events, "Save settings pointer action", || {
        !ui.get_busy()
            && !ui.get_dirty()
            && prefs(&ui).is_ok_and(|p| {
                p.desktop.time_zone == "UTC"
                    && p.reminders.audio_device == "synthetic-pinned-endpoint"
                    && p.reminders.animation
                    && p.reminders.animation_intensity == AnimationIntensity::Subtle
            })
    })?;
    ui.set_page(7);
    ui.set_update_status("".into());
    render(&window, &output.join("help-before.png"))?;
    click(&ui, 285., 352.);
    wait(&events, "diagnostic preview pointer action", || {
        ui.get_profile_review() == "diagnostic" && ui.get_data_operation_active()
    })?;
    render(&window, &output.join("diagnostic-preview.png"))?;
    click(&ui, 363., 570.);
    wait(&events, "diagnostic Close pointer action enabled", || {
        ui.get_profile_review().is_empty() && !ui.get_data_operation_active()
    })?;
    let focused = focused_states(&ui, &window, &output, &visual_pack)?;
    ui.hide()?;
    drop(backend);
    let store = Sandbox::open(&data)?;
    let saved = store.load()?;
    let state = store.load_state()?;
    if saved.desktop.time_zone != "UTC"
        || saved.reminders.audio_device != "synthetic-pinned-endpoint"
        || !saved.reminders.animation
        || saved.reminders.animation_intensity != AnimationIntensity::Subtle
        || state.pause_until.is_some()
        || state.pause_forever
        || saved.schedule.break_every != 720
        || saved.schedule.exercise_after_minutes != 1440
        || saved.idle_minutes != 120
    {
        return Err("Persisted controls/state differ from accepted UI selections".into());
    }
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"real_pointer_keyboard":true,"actual_backend_save":true,"native_windows_created":false,"desktop_input":false,"audio_played":false,"device_enumeration":false,"pause_calls":*pauses.lock().unwrap(),"audio_calls":*audio_calls.lock().unwrap(),"timezone_id_saved":true,"audio_id_saved":true,"animation_reenabled_and_saved":true,"diagnostic_close_enabled":true,"focused_states":focused,"new_character_has_no_media_controls":true,"visual_character_has_reference_and_layout_controls":true}),
        )?,
    )?;
    println!("Compact controls pointer/keyboard and persistence: passed");
    Ok(())
}
