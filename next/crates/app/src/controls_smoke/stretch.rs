//! Controlled service + SQLite + real Slint layout; no native windows or audio.
use super::*;
use crate::service::RuntimeView;
use kavychka_core::engine::Clock;
use std::cell::RefCell;

pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("new absolute evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut prefs = store.load()?;
    prefs.desktop.setup_complete = true;
    prefs.desktop.automatic_updates = false;
    prefs.schedule.work_days.clear();
    prefs.generation.auto_daily = false;
    store.save(&prefs)?;
    drop(store);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_audio_refreshing(true);
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let utc = "2026-10-29T09:00:00Z".parse::<chrono::DateTime<chrono::Utc>>()?;
    let backend = Backend::start_controlled(
        data.clone(),
        ui.as_weak(),
        Clock {
            utc,
            date: utc.date_naive(),
            mono: 1000,
        },
    );
    crate::wire(&ui, &backend);
    let latest = Rc::new(RefCell::new(None::<RuntimeView>));
    let view = latest.clone();
    let weak = ui.as_weak();
    ui.on_runtime_update(move |json| {
        let runtime: RuntimeView = serde_json::from_str(json.as_str()).unwrap();
        if let Some(ui) = weak.upgrade() {
            crate::shell::apply_runtime_status(&ui, &runtime);
        }
        *view.borrow_mut() = Some(runtime);
    });
    let sender = backend.sender.clone();
    ui.on_stretch_now(move || {
        sender.try_send(Command::Stretch).unwrap();
    });
    let sender = backend.sender.clone();
    ui.on_cancel_stretch(move || {
        sender.try_send(Command::CancelStretch).unwrap();
    });
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    wait(&events, "initial runtime view", || {
        ui.get_ready() && latest.borrow().is_some()
    })?;
    ui.set_page(0);
    if ui.get_exercise_active() {
        return Err("cancel unexpectedly enabled before any stretch".into());
    }
    render(&window, &output.join("today-idle.png"))?;
    ui.invoke_stretch_now();
    wait(&events, "manual stretch active", || {
        ui.get_exercise_active() && !ui.get_pending_key().is_empty()
    })?;
    std::thread::sleep(Duration::from_millis(250));
    pump(&events);
    for (width, height) in [(1100, 780), (720, 540)] {
        window.set_size(slint::PhysicalSize::new(width, height));
        for language in [0, 1] {
            ui.set_language_index(language);
            render(
                &window,
                &output.join(format!("today-active-{width}-lang{language}.png")),
            )?;
        }
    }
    // Actual pointer click in the software surface, not desktop input. This
    // verifies that the button is enabled and reaches the service callback.
    click(&ui, 390., 449.);
    wait(&events, "stretch cancelled", || {
        !ui.get_exercise_active() && ui.get_pending_key().is_empty()
    })?;
    if latest.borrow().as_ref().unwrap().audio_stop_requests != 1 {
        return Err("cancel did not request audio stop".into());
    }
    // The post-breakfast timer is cancellable before it has shown a message.
    backend.sender.try_send(Command::Breakfast)?;
    wait(&events, "breakfast timer armed", || {
        ui.get_exercise_active() && ui.get_breakfast_done()
    })?;
    ui.invoke_cancel_stretch();
    wait(&events, "breakfast timer cancelled", || {
        !ui.get_exercise_active()
    })?;
    let database = data.join("shell.sqlite3");
    let before_idle = kavychka_maintenance::digest(&database).map_err(|e| e.to_string())?;
    let idle_revision = latest.borrow().as_ref().unwrap().revision;
    let idle_until = Instant::now() + Duration::from_millis(800);
    while Instant::now() < idle_until {
        pump(&events);
        std::thread::sleep(Duration::from_millis(10));
    }
    if kavychka_maintenance::digest(&database).map_err(|e| e.to_string())? != before_idle {
        return Err("idle service rewrote unchanged SQLite state".into());
    }
    if latest.borrow().as_ref().unwrap().revision != idle_revision {
        return Err("idle service republished an unchanged runtime view".into());
    }
    render(&window, &output.join("today-cancelled.png"))?;
    ui.hide()?;
    drop(backend);
    let store = Sandbox::open(&data)?;
    let state = store.load_state()?;
    if !state.pending.is_empty()
        || state.exercise.is_some()
        || state.cycles.iter().any(|c| !c.finished)
    {
        return Err("cancellation not committed to SQLite".into());
    }
    if !store.load()?.schedule.exercise_enabled || state.breakfast_dates.is_empty() {
        return Err("cancel changed schedule or breakfast completion".into());
    }
    if !store.phrases()?.is_empty() || fs::read_dir(data.join("jobs"))?.next().is_some() {
        return Err("manual stretch unexpectedly started generation".into());
    }
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed": true, "exe_sha256": kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e|e.to_string())?,
            "native_windows": false, "desktop_input": false, "audio_played": false,
            "controlled_service": true, "cancel_committed": true, "audio_stop_requested": true,
            "breakfast_timer_cancelled": true, "schedule_preserved": true, "generation_not_started": true,
            "idle_database_unchanged": true,
            "idle_runtime_view_unchanged": true,
            "sizes": ["1100x780", "720x540"], "languages": ["ru", "en"]
        }))?,
    )?;
    Ok(())
}
