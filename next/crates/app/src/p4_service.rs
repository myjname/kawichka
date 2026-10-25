//! End-to-end Engine/service/storage/Slint checks using injected OS states.
//! Native windows are real; injected restrictions are never called physical tests.
use crate::*;
use kavychka_core::{
    engine::{Clock, Environment, NotificationState, PowerSource},
    reminders::{Category, Event},
};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
fn clock(second: i64) -> Clock {
    let utc = "2026-10-23T11:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap()
        + chrono::Duration::seconds(second);
    Clock {
        utc,
        mono: 2_000_000 + second as u64 * 1000,
        date: utc.date_naive(),
    }
}
fn event(category: Category, id: &str, second: i64) -> Event {
    Event {
        id: id.into(),
        category,
        due: clock(second).utc,
        followup: false,
    }
}
fn send(shell: &shell::Shell, second: i64, env: Environment, events: Vec<Event>) {
    shell.send(Command::ControlledStep {
        clock: clock(second),
        environment: env,
        events: Some(events),
        input: vec![],
    });
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
pub fn run(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if data.exists() {
        return Err("new P4 fixture required".into());
    }
    std::fs::create_dir_all(&output)?;
    {
        let mut store = Sandbox::open(&data)?;
        let mut prefs = store.load()?;
        prefs.desktop.setup_complete = true;
        prefs.desktop.automatic_updates = false;
        prefs.schedule.work_days.clear();
        prefs.generation.auto_daily = false;
        prefs.speech_language = Language::En;
        prefs.reminders.audio = AudioMode::Silent;
        prefs.reminders.fullscreen_visual = VisualMode::Both;
        store.save(&prefs)?;
    }
    shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_qualification(data.clone(), ui.as_weak(), clock(0));
    wire(&ui, &backend);
    let companion = shell::Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let shell = companion.clone();
    let outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    let started = Instant::now();
    let mut phase = 0;
    let restricted = Environment {
        notifications: NotificationState::Blocked,
        ..Default::default()
    };
    let battery = Environment {
        power: PowerSource::Battery,
        ..Default::default()
    };
    let mut request_count = 0;
    let mut stop_count = 0;
    let mut phrase_count = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(80),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let pass = (|| -> Result<(), String> {
                if started.elapsed() > Duration::from_secs(30) {
                    return Err(format!(
                        "P4 service timeout at {phase}: {} / {}",
                        ui.get_status(),
                        ui.get_library_status()
                    ));
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string());
                }
                let Some(view) = shell.view.borrow().clone() else {
                    return Ok(());
                };
                let at = |s| view.controlled_time == Some(clock(s).utc.timestamp_millis());
                match phase {
                    0 if ui.get_ready() => {
                        require(!ui.get_allow_during_dnd(), "DND exception must default off")?;
                        send(
                            &shell,
                            0,
                            Environment::default(),
                            vec![
                                event(Category::Lunch, "meal", 0),
                                event(Category::Break, "break", 0),
                            ],
                        );
                        phase = 1;
                    }
                    1 if view.notice.as_ref().is_some_and(|n| n.events.len() == 2) => {
                        require(
                            shell.message.window().is_visible(),
                            "combined native message missing",
                        )?;
                        request_count = view.presentation_requests;
                        stop_count = view.audio_stop_requests;
                        send(&shell, 1, restricted, vec![]);
                        phase = 2;
                    }
                    2 if at(1) => {
                        require(
                            view.notice.is_none() && !shell.message.window().is_visible(),
                            "restriction did not hide initiative",
                        )?;
                        require(
                            view.audio_stop_requests > stop_count,
                            "service did not call audio stop",
                        )?;
                        send(
                            &shell,
                            2,
                            restricted,
                            vec![event(Category::Dinner, "suppressed", 2)],
                        );
                        phase = 3;
                    }
                    3 if at(2) => {
                        require(
                            view.handled == 2 && view.skipped == 1,
                            "suppressed event was delivered",
                        )?;
                        send(
                            &shell,
                            3,
                            Environment::default(),
                            vec![event(Category::Dinner, "suppressed", 2)],
                        );
                        phase = 4;
                    }
                    4 if at(3) => {
                        require(
                            view.presentation_requests == request_count && view.notice.is_none(),
                            "suppression produced a replay/backlog",
                        )?;
                        ui.set_allow_during_dnd(true);
                        ui.invoke_edited();
                        ui.invoke_save();
                        phase = 5;
                    }
                    5 if !ui.get_busy() && view.prefs.reminders.allow_during_dnd => {
                        send(
                            &shell,
                            4,
                            restricted,
                            vec![event(Category::Exercise, "exception", 4)],
                        );
                        phase = 6;
                    }
                    6 if at(4) => {
                        require(
                            view.notice.is_some() && shell.message.window().is_visible(),
                            "saved exception not applied to actual service",
                        )?;
                        require(
                            view.presentation_requests == request_count + 1,
                            "exception generated wrong delivery count",
                        )?;
                        send(
                            &shell,
                            5,
                            Environment {
                                session_unknown: true,
                                ..restricted
                            },
                            vec![event(Category::Dinner, "disconnected", 5)],
                        );
                        phase = 7;
                    }
                    7 if at(5) => {
                        require(
                            view.notice.is_none() && view.handled == 3,
                            "exception bypassed unknown session",
                        )?;
                        send(
                            &shell,
                            6,
                            Environment {
                                locked: true,
                                ..restricted
                            },
                            vec![event(Category::Dinner, "locked", 6)],
                        );
                        phase = 8;
                    }
                    8 if at(6) => {
                        require(
                            view.notice.is_none() && view.handled == 3,
                            "exception bypassed lock",
                        )?;
                        send(&shell, 7, restricted, vec![]);
                        shell.send(Command::Pause(1));
                        phase = 9;
                    }
                    9 if at(7) && view.paused => {
                        send(
                            &shell,
                            8,
                            restricted,
                            vec![event(Category::Dinner, "paused", 8)],
                        );
                        phase = 10;
                    }
                    10 if at(8) => {
                        require(
                            view.notice.is_none() && view.handled == 3,
                            "exception bypassed app pause",
                        )?;
                        shell.send(Command::Pause(0));
                        send(&shell, 9, Environment::default(), vec![]);
                        phase = 11;
                    }
                    11 if at(9) && !view.paused => {
                        ui.set_fullscreen_visual_index(2);
                        ui.set_fullscreen_audio_index(3);
                        ui.invoke_edited();
                        ui.invoke_save();
                        phase = 12;
                    }
                    12 if !ui.get_busy()
                        && view.prefs.reminders.fullscreen_visual == VisualMode::Nothing =>
                    {
                        send(
                            &shell,
                            10,
                            Environment {
                                fullscreen: true,
                                ..restricted
                            },
                            vec![event(Category::Dinner, "fullscreen", 10)],
                        );
                        phase = 13;
                    }
                    13 if at(10) => {
                        require(
                            view.notice.is_none() && view.handled == 3,
                            "exception bypassed fullscreen policy",
                        )?;
                        send(
                            &shell,
                            11,
                            Environment {
                                notifications: NotificationState::Unknown,
                                ..Default::default()
                            },
                            vec![event(Category::Dinner, "unknown", 11)],
                        );
                        phase = 14;
                    }
                    14 if at(11) => {
                        require(
                            view.notice.is_none() && view.handled == 3,
                            "unknown notification permission treated as allowed",
                        )?;
                        send(
                            &shell,
                            12,
                            battery,
                            vec![event(Category::Dinner, "battery-reminder", 12)],
                        );
                        phase = 15;
                    }
                    15 if at(12) => {
                        require(
                            view.notice.is_some() && view.on_battery,
                            "battery stopped base reminders",
                        )?;
                        ui.set_batch_count(2);
                        ui.set_category_break(false);
                        ui.set_category_breakfast(false);
                        ui.set_category_lunch(true);
                        ui.set_category_dinner(false);
                        ui.set_category_exercise(false);
                        ui.invoke_edited();
                        ui.invoke_save();
                        phase = 16;
                    }
                    16 if !ui.get_busy() && view.prefs.generation.batch_count == 2 => {
                        let library: models_library::View =
                            serde_json::from_str(ui.get_library_json().as_str())
                                .map_err(|e| e.to_string())?;
                        phrase_count = library.phrases.len();
                        ui.invoke_generate_batch();
                        phase = 17;
                    }
                    17 if ui.get_library_status().contains("Connect AC power") => {
                        ui.invoke_generate_on_battery();
                        phase = 18;
                    }
                    18 => {
                        let library: models_library::View =
                            serde_json::from_str(ui.get_library_json().as_str())
                                .map_err(|e| e.to_string())?;
                        if library.phrases.len() == phrase_count + 2 {
                            require(
                                view.prefs.generation.ac_only,
                                "one task changed the persistent power preference",
                            )?;
                            ui.invoke_generate_batch();
                            phase = 19;
                        }
                    }
                    19 if ui.get_library_status().contains("Connect AC power") => {
                        send(&shell, 13, Environment::default(), vec![]);
                        shell.send(Command::Pause(1));
                        phase = 20;
                    }
                    20 if at(13) && view.paused => {
                        ui.invoke_generate_on_battery();
                        phase = 21;
                    }
                    21 if ui.get_library_status().contains("Generation paused") => {
                        require(!view.audio_busy, "silent fixture unexpectedly busy")?;
                        *result.borrow_mut() = Some(Ok(()));
                        ui.invoke_quit();
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = pass {
                *result.borrow_mut() = Some(Err(error));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    timer.stop();
    drop(timer);
    ui.hide()?;
    drop(companion);
    drop(backend);
    outcome
        .borrow_mut()
        .take()
        .ok_or("P4 service stopped early")?
        .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
    let store = Sandbox::open(&data)?;
    let state = store.load_state()?;
    for key in [
        "suppressed",
        "disconnected",
        "locked",
        "paused",
        "fullscreen",
        "unknown",
    ] {
        if !state.skipped.contains_key(key) || state.handled.contains_key(key) {
            return Err(format!("suppressed event {key} not persisted correctly").into());
        }
    }
    if !store.load()?.reminders.allow_during_dnd || !store.load()?.generation.ac_only {
        return Err("settings not persisted".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"actual_service_engine_storage_and_native_windows":true,"dnd_default_exception_stop_hide_no_replay":true,"lock_unknown_session_pause_fullscreen_independent":true,"notification_unknown_conservative":true,"base_reminders_on_battery":true,"explicit_one_batch_override_then_blocked_again":true,"override_never_bypasses_pause":true,"persistent_power_preference_unchanged":true,"python_or_models_used":false,"clock_and_os_states_injected":true,"physical_transitions_tested":false,"user_library_opened":false}),
        )?,
    )?;
    println!("P4 service/policy/native-window integration passed.");
    Ok(())
}
