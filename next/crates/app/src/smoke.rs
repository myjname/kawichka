//! Drives this application's own Slint window, never the user's desktop.
use super::*;
use slint::{
    LogicalPosition, Model,
    platform::{
        EventLoopProxy, Platform, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Events = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;
struct TestPlatform {
    window: Rc<MinimalSoftwareWindow>,
    events: Events,
    quit: Arc<AtomicBool>,
}
struct Proxy {
    events: Events,
    quit: Arc<AtomicBool>,
}
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.quit.store(true, Ordering::Relaxed);
        Ok(())
    }
    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), slint::EventLoopError> {
        self.events.lock().unwrap().push_back(event);
        Ok(())
    }
}
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy {
            events: self.events.clone(),
            quit: self.quit.clone(),
        }))
    }
}
fn pump(events: &Events) {
    loop {
        let event = events.lock().unwrap().pop_front();
        match event {
            Some(event) => event(),
            None => break,
        }
    }
    slint::platform::update_timers_and_animations();
}
fn wait(events: &Events, condition: impl Fn() -> bool) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pump(events);
        if condition() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("UI smoke timeout".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(ui: &AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn render(
    window: &MinimalSoftwareWindow,
    output: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Let standard-widget state transitions settle before recording evidence.
    std::thread::sleep(Duration::from_millis(200));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(
        window.size().width,
        window.size().height,
    );
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), window.size().width as usize);
    });
    image::save_buffer(
        output,
        pixels.as_bytes(),
        window.size().width,
        window.size().height,
        image::ColorType::Rgb8,
    )?;
    Ok(())
}

/// Uses a real Winit/software-rendered window and real event-loop dispatch.
/// Input below is routed only to this test's window, never through desktop automation.
pub fn run_native(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if data.try_exists()? && std::fs::read_dir(&data)?.next().is_some() {
        return Err("native smoke requires an empty data directory".into());
    }
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into());
    #[cfg(windows)]
    let selector =
        { selector.with_winit_window_attributes_hook(|attributes| attributes.with_active(false)) };
    selector.select()?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    ui.set_system_dark(kavychka_platform::dark_preference().unwrap_or(false));
    {
        let mut store = Sandbox::open(&data)?;
        let mut prefs = store.load()?;
        prefs.desktop.setup_complete = true;
        store.save(&prefs)?;
    }
    let backend = Backend::start(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    std::fs::create_dir_all(&output)?;
    let outcome = Rc::new(std::cell::RefCell::new(None::<Result<(), String>>));
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let result = outcome.clone();
    let evidence = output.clone();
    let started = Instant::now();
    let mut phase = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(300),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let step = (|| -> Result<(), Box<dyn std::error::Error>> {
                if started.elapsed() > Duration::from_secs(12) {
                    return Err("native smoke timed out".into());
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string().into());
                }
                match phase {
                    0 if ui.get_ready() => {
                        native_snapshot(&ui, &evidence.join("overview.png"))?;
                        click(&ui, 100., 122.);
                        phase = 1;
                    }
                    1 => {
                        if ui.get_page() != 1 {
                            return Err("native navigation failed".into());
                        }
                        native_snapshot(&ui, &evidence.join("schedule.png"))?;
                        ui.set_theme_index(2);
                        ui.set_language_index(1);
                        ui.set_user_name("Нативная проверка".into());
                        ui.set_start_time("09:45".into());
                        ui.invoke_edited();
                        ui.set_page(5);
                        ui.invoke_save();
                        phase = 2;
                    }
                    2 if !ui.get_busy() => {
                        if ui.get_dirty() {
                            return Err("native save failed".into());
                        }
                        native_snapshot(&ui, &evidence.join("settings.png"))?;
                        ui.set_user_name("Несохранённое".into());
                        ui.invoke_edited();
                        ui.window().dispatch_event(WindowEvent::CloseRequested);
                        if !ui.get_confirm_close() || !ui.window().is_visible() {
                            return Err("native close guard failed".into());
                        }
                        phase = 3;
                    }
                    3 => {
                        native_snapshot(&ui, &evidence.join("unsaved-close.png"))?;
                        ui.set_confirm_close(false);
                        ui.invoke_discard();
                        phase = 4;
                    }
                    4 if !ui.get_busy() => {
                        if ui.get_dirty() || ui.get_user_name() != "Нативная проверка"
                        {
                            return Err("native discard failed".into());
                        }
                        *result.borrow_mut() = Some(Ok(()));
                        ui.invoke_quit();
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(error) = step {
                *result.borrow_mut() = Some(Err(error.to_string()));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.run()?;
    drop(timer);
    drop(backend);
    outcome
        .borrow_mut()
        .take()
        .ok_or("native event loop stopped prematurely")?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let store = Sandbox::open(&data)?;
    let saved = store.load()?;
    if saved.user_name != "Нативная проверка" || saved.schedule.work_start != 585 {
        return Err("native persistence failed".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"real_winit_window":true,"software_renderer":true,"navigation":true,
            "async_persistence":true,"close_guard":true,"discard":true,"clean_exit":true,
            "os_close_message_tested":false,"dpi_matrix_tested":false,"accessibility_audited":false,
            "desktop_input_injected":false,"live_storage_opened":false
        }))?,
    )?;
    println!("Native Slint shell smoke passed.");
    Ok(())
}
fn native_snapshot(
    ui: &AppWindow,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let pixels = ui.window().take_snapshot()?;
    image::save_buffer(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
pub fn run(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    run_impl(data, output, true)
}
/// Public callbacks + real service/storage, using an in-memory software window.
/// No HWND, global input, desktop changes, Python or audio playback.
/// Capture the real compact UI in an isolated software renderer. No OS windows,
/// desktop input, installation or model downloads are performed by this catalog.
fn capture_review_matrix(
    ui: &AppWindow,
    window: &MinimalSoftwareWindow,
    events: &Events,
    output: &std::path::Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let folder = output.join("matrix");
    std::fs::create_dir(&folder)?;
    let pages = [
        "today",
        "schedule",
        "library",
        "models",
        "character",
        "settings",
        "data",
        "help",
    ];
    let initial_page = ui.get_page();
    let initial_language = ui.get_language_index();
    let initial_theme = ui.get_theme_index();
    let mut evidence = Vec::new();
    for (width, height) in [(1100, 780), (720, 540)] {
        window.set_size(slint::PhysicalSize::new(width, height));
        for language in 0..2 {
            ui.set_language_index(language);
            ui.invoke_edited();
            ui.invoke_save();
            wait(events, || !ui.get_busy() && !ui.get_failed())?;
            for theme in [1, 2] {
                ui.set_theme_index(theme);
                let tag = format!(
                    "{}-{}-{width}x{height}",
                    if language == 0 { "ru" } else { "en" },
                    if theme == 1 { "light" } else { "dark" }
                );
                for (page, name) in pages.iter().enumerate() {
                    ui.set_page(page as i32);
                    ui.set_scroll_y(0.);
                    pump(events);
                    let filename = format!("{name}-{tag}-top.png");
                    render(window, &folder.join(&filename))?;
                    evidence.push(filename);
                    if ui.get_scroll_overflow() > 2. {
                        ui.set_scroll_y(-ui.get_scroll_overflow());
                        let filename = format!("{name}-{tag}-bottom.png");
                        render(window, &folder.join(&filename))?;
                        evidence.push(filename);
                    }
                }
                for (page, name) in [
                    (2, "library"),
                    (3, "models"),
                    (4, "character"),
                    (6, "data"),
                    (7, "help"),
                ] {
                    ui.set_page(page);
                    ui.set_library_controls(page == 2);
                    ui.set_model_advanced(page == 3);
                    ui.set_character_options(page == 4);
                    ui.set_preset_management(page == 4);
                    ui.set_delivery_editor(page == 4);
                    ui.set_data_advanced(page == 6);
                    if page == 2
                        && let Some(row) = ui.get_library_items().row_data(0)
                    {
                        ui.set_expanded_phrase(row.identifier);
                    }
                    if page == 7 {
                        ui.set_profile_review("diagnostic".into());
                        ui.set_profile_summary_ru("Пример локальной диагностики для проверки интерфейса. Данные не отправляются.".into());
                        ui.set_profile_summary_en(
                            "Local diagnostic preview for UI review. No data is sent.".into(),
                        );
                    }
                    ui.set_scroll_y(0.);
                    pump(events);
                    let filename = format!("{name}-{tag}-expanded-top.png");
                    render(window, &folder.join(&filename))?;
                    evidence.push(filename);
                    if ui.get_scroll_overflow() > 2. {
                        ui.set_scroll_y(-ui.get_scroll_overflow());
                        let filename = format!("{name}-{tag}-expanded-bottom.png");
                        render(window, &folder.join(&filename))?;
                        evidence.push(filename);
                    }
                    ui.set_library_controls(false);
                    ui.set_model_advanced(false);
                    ui.set_character_options(false);
                    ui.set_preset_management(false);
                    ui.set_delivery_editor(false);
                    ui.set_data_advanced(false);
                    ui.set_expanded_phrase("".into());
                    ui.set_profile_review("".into());
                }
                ui.set_page(3);
                ui.set_scroll_y(0.);
                for (state, progress, error) in [
                    ("working", -1., false),
                    ("progress", 0.4, false),
                    ("error", -1., true),
                ] {
                    ui.set_generation_running(!error);
                    ui.set_task_progress(progress);
                    ui.set_task_error(error);
                    ui.set_task_title(
                        (if language == 0 {
                            "Модели · пример"
                        } else {
                            "Models · preview"
                        })
                        .into(),
                    );
                    ui.set_library_status(
                        (if language == 0 {
                            if error {
                                "Проверка не прошла. Открой подробности."
                            } else {
                                "Проверяем необходимые компоненты"
                            }
                        } else if error {
                            "Check failed. Open details."
                        } else {
                            "Checking required components"
                        })
                        .into(),
                    );
                    let filename = format!("tasks-{state}-{tag}.png");
                    render(window, &folder.join(&filename))?;
                    evidence.push(filename);
                }
                ui.set_generation_running(false);
                ui.set_task_error(false);
                ui.set_task_progress(-1.);
                ui.set_task_title("".into());
                ui.set_library_status("".into());
                ui.set_onboarding(true);
                for step in 0..4 {
                    ui.set_setup_step(step);
                    ui.set_onboarding_scroll_y(0.);
                    pump(events);
                    let filename = format!("onboarding-{step}-{tag}.png");
                    render(window, &folder.join(&filename))?;
                    evidence.push(filename);
                    if ui.get_onboarding_scroll_overflow() > 2. {
                        ui.set_onboarding_scroll_y(-ui.get_onboarding_scroll_overflow());
                        let filename = format!("onboarding-{step}-{tag}-bottom.png");
                        render(window, &folder.join(&filename))?;
                        evidence.push(filename);
                    }
                }
                ui.set_onboarding(false);
            }
        }
    }
    ui.set_language_index(initial_language);
    ui.set_theme_index(initial_theme);
    ui.invoke_edited();
    ui.invoke_save();
    wait(events, || !ui.get_busy() && !ui.get_failed())?;
    ui.set_page(initial_page);
    ui.set_scroll_y(0.);
    window.set_size(slint::PhysicalSize::new(1100, 850));
    pump(events);
    std::fs::write(
        folder.join("index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "files": evidence, "native_windows_created": false, "desktop_input_sent": false,
            "manual_acceptance": false, "languages": ["ru", "en"], "themes": ["light", "dark"],
            "sizes": [[1100, 780], [720, 540]], "real_app_components": true
        }))?,
    )?;
    Ok(evidence.len())
}

pub fn run_review(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use crate::models_library::View;
    use kavychka_core::{
        engine::{Clock, Engine, Environment},
        reminders::{Category, Event},
    };
    use slint::Model;
    if !output.is_absolute() || output.exists() {
        return Err("new absolute review fixture required".into());
    }
    std::fs::create_dir_all(&output)?;
    let data = output.join("data");
    let clock = |s: i64| {
        let utc = "2026-01-01T09:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            + chrono::Duration::seconds(s);
        Clock {
            utc,
            mono: 1_000_000 + s as u64 * 1000,
            date: utc.date_naive(),
        }
    };
    {
        let mut store = Sandbox::open(&data)?;
        let mut p = Preferences::default();
        p.desktop.setup_complete = true;
        p.desktop.automatic_updates = false;
        p.desktop.time_zone = "UTC".into();
        p.generation.auto_daily = false;
        p.generation.ac_only = false;
        p.schedule.work_days.clear();
        p.repeat_minutes = 1;
        p.reminders.audio = AudioMode::Silent;
        for i in 0..206 {
            let day = if i == 205 {
                (clock(0).date - chrono::Days::new(1)).to_string()
            } else {
                clock(0).date.to_string()
            };
            store.accept_phrase(
                &kavychka_storage::library::Phrase {
                    id: format!("review-{i:04}"),
                    text: format!("Пообедай: синтетическая запись {i}."),
                    category: "lunch".into(),
                    model: "system".into(),
                    profile: "system".into(),
                    day: Some(day),
                    audio_path: None,
                    approved: true,
                    deleted: false,
                    rejected: false,
                    used_at: None,
                    created_at: clock(0).utc.to_rfc3339(),
                    followup: false,
                    language: "ru".into(),
                    seed: 42,
                    voice_style: String::new(),
                    voice_instruction: String::new(),
                    voice_reference: String::new(),
                },
                None,
            )?;
        }
        let mut engine = Engine::new(p.clone(), Default::default(), clock(0))?;
        for _ in 0..200 {
            engine.manual_stretch(clock(0));
        }
        let mut old = std::mem::take(&mut engine.archived_pending);
        old.extend(engine.state.pending);
        engine.state.pending = old;
        engine.state.checkpoint_utc = Some(clock(0).utc);
        store.save_runtime(&p, &engine.state)?;
    }
    let window = MinimalSoftwareWindow::new(Default::default());
    let events: Events = Default::default();
    let quit = Arc::new(AtomicBool::new(false));
    slint::platform::set_platform(Box::new(TestPlatform {
        window: window.clone(),
        events: events.clone(),
        quit,
    }))?;
    let ui = AppWindow::new()?;
    ui.window().set_size(slint::LogicalSize::new(1100., 850.));
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_controlled(data.clone(), ui.as_weak(), clock(0));
    wire(&ui, &backend);
    let latest = Rc::new(std::cell::RefCell::new(None::<service::RuntimeView>));
    let view = latest.clone();
    let runtime_ui = ui.as_weak();
    ui.on_runtime_update(move |json| {
        let parsed = serde_json::from_str::<service::RuntimeView>(json.as_str()).ok();
        if let Some(runtime) = parsed.as_ref()
            && let Some(ui) = runtime_ui.upgrade()
        {
            shell::apply_runtime_status(&ui, runtime);
        }
        *view.borrow_mut() = parsed;
    });
    ui.show()?;
    let library = || serde_json::from_str::<View>(ui.get_library_json().as_str()).ok();
    wait(&events, || ui.get_ready() && ui.get_archive_total() == 72)?;
    assert_eq!(ui.get_library_items().row_count(), 50);
    assert_eq!(library().unwrap().total, 206);
    ui.set_library_today(true);
    ui.invoke_library_filter();
    wait(&events, || library().is_some_and(|v| v.total == 205))?;
    for page in 1..=4 {
        ui.invoke_library_page(1);
        wait(&events, || ui.get_library_page_index() == page)?;
    }
    assert_eq!(ui.get_library_items().row_count(), 5);
    assert!(!ui.get_library_has_next());
    let last_id = ui.get_library_items().row_data(4).unwrap().identifier;
    ui.invoke_select_phrase(last_id.clone(), false);
    wait(&events, || {
        library().is_some_and(|v| {
            v.phrases
                .iter()
                .any(|r| r.id == last_id.as_str() && r.rejected)
        })
    })?;
    ui.set_page(2);
    render(&window, &output.join("library-last-page-ru.png"))?;
    let archived = ui.get_archived_notices().row_data(0).unwrap().identifier;
    ui.invoke_confirm_archived(archived.clone());
    wait(&events, || ui.get_archive_total() == 71)?;
    ui.invoke_archive_page(1);
    wait(&events, || ui.get_archive_page_index() == 1)?;
    ui.set_page(0);
    render(&window, &output.join("pending-history-ru.png"))?;
    let event = |category, id: &str, s| Event {
        id: id.into(),
        category,
        due: clock(s).utc,
        followup: false,
    };
    backend.sender.send(Command::ControlledStep {
        clock: clock(1),
        environment: Environment::default(),
        events: Some(vec![event(Category::Exercise, "review-exercise", 1)]),
        input: vec![],
    })?;
    wait(&events, || {
        latest.borrow().as_ref().is_some_and(|v| {
            v.notice
                .as_ref()
                .is_some_and(|n| n.cycle == "review-exercise")
        })
    })?;
    ui.set_stretch(false);
    ui.invoke_edited();
    ui.invoke_save();
    wait(&events, || {
        !ui.get_busy()
            && !ui.get_failed()
            && latest
                .borrow()
                .as_ref()
                .is_some_and(|v| !v.prefs.schedule.exercise_enabled)
    })?;
    backend.sender.send(Command::ControlledStep {
        clock: clock(65),
        environment: Environment::default(),
        events: Some(vec![]),
        input: vec![],
    })?;
    wait(&events, || {
        latest
            .borrow()
            .as_ref()
            .is_some_and(|v| v.controlled_time == Some(clock(65).utc.timestamp_millis()))
    })?;
    assert!(latest.borrow().as_ref().unwrap().notice.is_none());
    backend.sender.send(Command::ControlledStep {
        clock: clock(70),
        environment: Environment::default(),
        events: Some(vec![
            event(Category::Lunch, "review-lunch", 70),
            event(Category::Break, "review-break", 70),
        ]),
        input: vec![],
    })?;
    wait(&events, || {
        latest
            .borrow()
            .as_ref()
            .is_some_and(|v| v.notice.as_ref().is_some_and(|n| n.events.len() == 2))
    })?;
    assert_eq!(
        latest
            .borrow()
            .as_ref()
            .unwrap()
            .notice
            .as_ref()
            .unwrap()
            .text,
        kavychka_core::engine::builtin(Category::Lunch, true, 0, Language::Ru)
    );
    ui.set_language_index(1);
    ui.set_theme_index(1);
    ui.invoke_edited();
    ui.invoke_save();
    wait(&events, || {
        !ui.get_busy() && !ui.get_failed() && ui.get_english()
    })?;
    assert!(
        ui.get_archived_notices()
            .row_data(0)
            .unwrap()
            .caption
            .starts_with("2026-01-01")
    );
    render(&window, &output.join("pending-history-en-light.png"))?;
    let matrix_screenshots = capture_review_matrix(&ui, &window, &events, &output)?;
    let user_cancel = backend.cancellation.clone();
    drop(backend);
    assert!(
        !user_cancel.load(Ordering::Acquire),
        "shutdown must not become a user cancellation"
    );
    pump(&events);
    let store = Sandbox::open(&data)?;
    assert!(store.phrase(&last_id)?.rejected);
    assert!(
        !store
            .pending_archive_page(0)?
            .0
            .iter()
            .any(|n| n.key == archived.as_str())
    );
    assert!(store.phrases()?.iter().all(|p| p.used_at.is_none()));
    ui.hide()?;
    drop(ui);
    let installer = SetupWindow::new()?;
    installer.set_version(app_version().into());
    installer.set_program_path(r"E:\Apps\Kavychka".into());
    installer.set_data_path(r"E:\Kavychka data".into());
    installer.set_status("Готово к установке / Ready to install".into());
    installer.show()?;
    for dark in [false, true] {
        installer.set_dark(dark);
        for (width, height) in [(720, 590), (660, 540)] {
            window.set_size(slint::PhysicalSize::new(width, height));
            for advanced in [false, true] {
                installer.set_advanced(advanced);
                installer.set_scroll_y(0.);
                let tag = format!(
                    "{}-{width}x{height}{}",
                    if dark { "dark" } else { "light" },
                    if advanced { "-advanced" } else { "" }
                );
                render(&window, &output.join(format!("installer-{tag}.png")))?;
                if installer.get_scroll_overflow() > 2. {
                    installer.set_scroll_y(-installer.get_scroll_overflow());
                    render(&window, &output.join(format!("installer-{tag}-bottom.png")))?;
                }
            }
        }
    }
    installer.hide()?;
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"version":app_version(),"sha256":kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e|e.to_string())?,"fixture_date":clock(0).date,"rows":store.phrases()?.len(),"page_size":50,"last_today_page_rows":5,"history_preserved":true,"archive_acknowledgement":true,"exercise_disable":true,"combined_fallback":true,"native_desktop_used":false,"audio_played":false,"manual_acceptance":false,"matrix_screenshots":matrix_screenshots}),
        )?,
    )?;
    Ok(())
}
pub fn run_keyboard(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    run_impl(data, output, false)
}
fn run_impl(
    data: PathBuf,
    output: PathBuf,
    matrix: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Isolated smoke must never mutate preexisting data, including an earlier sandbox.
    if data.try_exists()? && std::fs::read_dir(&data)?.next().is_some() {
        return Err("smoke requires an empty data directory".into());
    }
    let window = MinimalSoftwareWindow::new(Default::default());
    let events = Events::default();
    let quit = Arc::new(AtomicBool::new(false));
    slint::platform::set_platform(Box::new(TestPlatform {
        window: window.clone(),
        events: events.clone(),
        quit: quit.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    wait(&events, || ui.get_ready() || ui.get_failed())?;
    if ui.get_failed() {
        return Err(ui.get_status().to_string().into());
    }
    std::fs::create_dir_all(&output)?;
    if !ui.get_onboarding() {
        return Err("fresh storage must show setup".into());
    }
    render(&window, &output.join("welcome.png"))?;
    ui.set_language_index(1);
    ui.set_speech_index(0);
    ui.set_address_index(1);
    ui.invoke_edited();
    ui.invoke_setup_next(1);
    if ui.get_setup_step() != 1 {
        return Err("setup next failed".into());
    }
    ui.set_start_time("25:99".into());
    ui.invoke_setup_next(1);
    if ui.get_setup_step() != 1 || !ui.get_failed() {
        return Err("setup accepted invalid schedule".into());
    }
    ui.set_start_time("09:00".into());
    ui.invoke_setup_next(1);
    if ui.get_setup_step() != 2 {
        return Err("setup sound step failed".into());
    }
    ui.invoke_setup_next(-1);
    ui.invoke_setup_next(1);
    if ui.get_address_index() != 1 || ui.get_language_index() != 1 {
        return Err("setup back lost draft".into());
    }
    render(&window, &output.join("setup-sound.png"))?;
    ui.invoke_setup_next(1);
    render(&window, &output.join("setup-dnd-limitation.png"))?;
    ui.invoke_finish_setup(false);
    wait(&events, || !ui.get_busy())?;
    if ui.get_failed() || ui.get_onboarding() || ui.get_dirty() {
        return Err("setup finish failed".into());
    }
    let completed: Preferences = serde_json::from_str(ui.get_loaded_prefs().as_str())?;
    if !completed.desktop.setup_complete || completed.generation.address_form != "female" {
        return Err("setup not committed".into());
    }
    ui.invoke_discard();
    wait(&events, || !ui.get_busy())?;
    if ui.get_onboarding() {
        return Err("setup repeated on reload".into());
    }
    ui.set_language_index(0);
    ui.set_animation_index(2);
    ui.set_audio_device_id("p3-missing-endpoint".into());
    ui.invoke_edited();
    ui.invoke_save();
    wait(&events, || !ui.get_busy())?;
    ui.invoke_refresh_audio();
    wait(&events, || ui.get_audio_device_index() > 0)?;
    if ui.get_audio_device_id() != "p3-missing-endpoint" {
        return Err("missing output silently replaced".into());
    }
    ui.set_category_break(false);
    ui.set_category_breakfast(false);
    ui.set_category_lunch(true);
    ui.set_category_dinner(false);
    ui.set_category_exercise(false);
    ui.set_path_model_index(0);
    ui.invoke_path_selected();
    ui.set_path_model_value("E:\\P3 text model".into());
    ui.invoke_path_edited();
    ui.set_path_model_index(8);
    ui.invoke_path_selected();
    ui.set_path_model_value("E:\\P3 voice model".into());
    ui.invoke_path_edited();
    ui.set_path_model_index(0);
    ui.invoke_path_selected();
    if ui.get_path_model_value() != "E:\\P3 text model" {
        return Err("switching path discarded edit".into());
    }
    ui.invoke_edited();
    ui.invoke_save();
    wait(&events, || !ui.get_busy())?;
    if ui.get_failed() {
        return Err(ui.get_status().to_string().into());
    }
    if !matrix {
        // P4's changed controls at the supported minimum size. This is a
        // software scale/layout check, not a physical monitor qualification.
        for scale in [1., 1.5, 2.] {
            ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
            window.set_size(slint::PhysicalSize::new(
                (720. * scale) as u32,
                (540. * scale) as u32,
            ));
            for language in 0..2 {
                for theme in [1, 2] {
                    ui.set_language_index(language);
                    ui.set_theme_index(theme);
                    ui.set_page(5);
                    pump(&events);
                    ui.set_scroll_y(-ui.get_scroll_overflow());
                    render(
                        &window,
                        &output.join(format!("p4-settings-{scale}-{language}-{theme}.png")),
                    )?;
                    ui.set_scroll_y((-ui.get_scroll_overflow() + 320.).min(0.));
                    render(
                        &window,
                        &output.join(format!("p4-dnd-{scale}-{language}-{theme}.png")),
                    )?;
                    ui.set_on_battery(true);
                    ui.set_page(2);
                    pump(&events);
                    ui.set_scroll_y(-520.);
                    render(
                        &window,
                        &output.join(format!("p4-battery-{scale}-{language}-{theme}.png")),
                    )?;
                }
            }
        }
        ui.set_on_battery(false);
        ui.set_language_index(0);
        ui.set_theme_index(1);
        ui.set_page(0);
        ui.set_scroll_y(0.);
        ui.window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1. });
        window.set_size(slint::PhysicalSize::new(1100, 780));
    }
    render(&window, &output.join("overview-light.png"))?;
    for page in 0..8 {
        click(&ui, 100., 86. + page as f32 * 36.);
        if ui.get_page() != page {
            return Err(format!("navigation click failed for {page}").into());
        }
        render(&window, &output.join(format!("page-{page}.png")))?;
    }
    for scale in if matrix { vec![1., 1.5, 2.] } else { vec![] } {
        ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(slint::PhysicalSize::new(
            (720. * scale) as u32,
            (540. * scale) as u32,
        ));
        for language in 0..2 {
            for theme in [1, 2] {
                for page in 0..8 {
                    ui.set_language_index(language);
                    ui.set_theme_index(theme);
                    ui.set_page(page);
                    render(
                        &window,
                        &output.join(format!("compact-{scale}-{language}-{theme}-{page}.png")),
                    )?;
                    ui.set_scroll_y(-ui.get_scroll_overflow());
                    render(
                        &window,
                        &output.join(format!(
                            "compact-{scale}-{language}-{theme}-{page}-bottom.png"
                        )),
                    )?;
                    ui.set_scroll_y(0.);
                }
            }
        }
    }
    ui.window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1. });
    window.set_size(slint::PhysicalSize::new(1100, 780));
    // Keyboard activation of custom navigation must work as well as pointer clicks.
    click(&ui, 100., 122.);
    ui.set_page(0);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    if ui.get_page() != 1 {
        return Err("keyboard navigation activation failed".into());
    }
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Tab.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Tab.into(),
    });
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    if ui.get_page() != 2 {
        return Err("Tab did not advance navigation focus".into());
    }
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Backtab.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Backtab.into(),
    });
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    if ui.get_page() != 1 {
        return Err("Shift+Tab did not restore navigation focus".into());
    }
    ui.set_theme_index(2);
    ui.set_language_index(1);
    ui.set_speech_index(0);
    ui.set_user_name("Алёна".into());
    ui.set_start_time("09:30".into());
    ui.set_break_minutes(90);
    ui.set_monday(false);
    ui.set_saturday(true);
    ui.set_idle_minutes(7);
    ui.invoke_edited();
    ui.set_page(5);
    render(&window, &output.join("settings-dark-en.png"))?;
    click(&ui, 986., 742.);
    if !ui.get_busy() {
        return Err("save button did not dispatch".into());
    }
    wait(&events, || !ui.get_busy())?;
    if ui.get_failed() || ui.get_dirty() {
        return Err("valid settings did not save".into());
    }
    ui.set_start_time("25:99".into());
    ui.invoke_edited();
    ui.invoke_save();
    if !ui.get_failed() || !ui.get_dirty() {
        return Err("invalid time was accepted".into());
    }
    render(&window, &output.join("invalid-settings.png"))?;
    ui.invoke_discard();
    wait(&events, || !ui.get_busy())?;
    if ui.get_start_time() != "09:30" || ui.get_dirty() || ui.get_failed() {
        return Err("discard lost committed settings".into());
    }
    ui.set_theme_index(0);
    ui.set_system_dark(true);
    pump(&events);
    if !ui.global::<Colors>().get_dark() {
        return Err("system dark theme ignored".into());
    }
    ui.set_theme_index(1);
    pump(&events);
    if ui.global::<Colors>().get_dark() {
        return Err("explicit light theme ignored".into());
    }
    ui.invoke_discard();
    wait(&events, || !ui.get_busy())?;
    click(&ui, 100., 122.); // Focus a known initiating item before the close request.
    ui.invoke_edited();
    ui.invoke_quit();
    if !ui.get_confirm_close() || quit.load(Ordering::Relaxed) {
        return Err("unsaved exit was not guarded".into());
    }
    pump(&events);
    for _ in 0..3 {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: slint::platform::Key::Tab.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: slint::platform::Key::Tab.into(),
        });
    }
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "\n".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: "\n".into() });
    if ui.get_confirm_close() || quit.load(Ordering::Relaxed) {
        return Err("modal Tab focus/default cancel failed".into());
    }
    ui.invoke_quit();
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: slint::platform::Key::Escape.into(),
    });
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: slint::platform::Key::Escape.into(),
    });
    if ui.get_confirm_close() || quit.load(Ordering::Relaxed) {
        return Err("Escape did not cancel modal".into());
    }
    ui.set_page(0);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    if ui.get_page() != 1 {
        return Err("modal cancellation did not restore initiating focus".into());
    }
    ui.invoke_quit();
    ui.invoke_discard_and_quit();
    if !quit.load(Ordering::Relaxed) {
        return Err("explicit exit did not quit".into());
    }
    // The headless proxy lets us verify the other exit path in the same isolated scene.
    quit.store(false, Ordering::Relaxed);
    ui.set_confirm_close(false);
    ui.set_end_time("25:99".into());
    ui.invoke_edited();
    ui.invoke_quit();
    ui.invoke_save_and_quit();
    if !ui.get_failed() || quit.load(Ordering::Relaxed) || ui.get_exit_after_save() {
        return Err("failed save must not exit".into());
    }
    ui.invoke_discard();
    wait(&events, || !ui.get_busy())?;
    ui.set_break_minutes(95);
    ui.invoke_edited();
    ui.invoke_quit();
    ui.invoke_save_and_quit();
    wait(&events, || quit.load(Ordering::Relaxed))?;
    if ui.get_failed() || ui.get_dirty() {
        return Err("save and exit did not commit".into());
    }
    ui.hide()?;
    drop(backend);
    let store = Sandbox::open(&data)?;
    let saved = store.load()?;
    if saved.theme != Theme::Dark
        || saved.ui_language != Language::En
        || saved.speech_language != Language::Ru
        || saved.user_name != "Алёна"
        || saved.schedule.work_start != 570
        || saved.schedule.break_every != 95
        || saved.schedule.work_days.contains(&1)
        || !saved.schedule.work_days.contains(&6)
        || saved.idle_minutes != 7
        || saved.reminders.audio_device != "p3-missing-endpoint"
        || !saved.reminders.idle_continuous
        || saved.generation.address_form != "female"
        || saved.generation.categories != ["lunch"]
        || saved.generation.model_paths.len() != 2
        || !saved.desktop.setup_complete
    {
        return Err("restart did not preserve settings".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"navigation_pages":8,"keyboard_activation":true,"async_save":true,
            "invalid_settings_rejected":true,"discard":true,"restart_persistence":true,
            "system_and_manual_theme":true,"independent_languages":true,"exit":true,
            "save_and_exit":true,"failed_save_does_not_exit":true,
            "onboarding_validation_back_finish_reload":true,"address_form":true,"pinned_missing_output_preserved":true,"model_paths_and_categories":true,"native_window_tested":false,"reminders_started":false,"live_storage_opened":false
        }))?,
    )?;
    println!(
        "Slint shell smoke passed; native window/DPI/accessibility and reminders remain separate checks."
    );
    Ok(())
}
