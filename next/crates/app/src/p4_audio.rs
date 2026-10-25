//! Native PCM cancellation through the production service. OS restrictions and
//! clock are injected; no lock, sleep, endpoint or desktop input is manipulated.
use crate::*;
use kavychka_core::{
    engine::{Clock, Environment, NotificationState},
    reminders::{Category, Event},
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
const CASES: &[&str] = &[
    "policy-dnd-voice",
    "policy-dnd-chime",
    "policy-pause-voice",
    "policy-lock-voice",
    "policy-fullscreen-voice",
    "policy-suspend-voice",
    "policy-session-voice",
    "policy-dnd-exception",
];
const EVENT: &str = "p4-policy-event";
pub fn supports(case: &str) -> bool {
    CASES.contains(&case)
}
fn clock(second: i64) -> Clock {
    let utc = "2026-10-23T12:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap()
        + chrono::Duration::seconds(second);
    Clock {
        utc,
        mono: 3_000_000 + second as u64 * 1000,
        date: utc.date_naive(),
    }
}
fn step(shell: &shell::Shell, second: i64, environment: Environment, events: Option<Vec<Event>>) {
    shell.send(Command::ControlledStep {
        clock: clock(second),
        environment,
        events,
        input: vec![],
    });
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if output.exists() {
        return Err("new native audio evidence required".into());
    }
    std::fs::create_dir_all(&output)?;
    let mut rows = vec![];
    for case in CASES {
        let data = output.join(case).join("data");
        {
            let mut store = Sandbox::open(&data)?;
            let mut prefs = store.load()?;
            prefs.desktop.setup_complete = true;
            prefs.desktop.automatic_updates = false;
            prefs.schedule.work_days.clear();
            prefs.generation.auto_daily = false;
            prefs.generation.runtime = false;
            prefs.speech_language = Language::En;
            prefs.reminders.allow_during_dnd = *case == "policy-dnd-exception";
            prefs.reminders.fullscreen_visual = VisualMode::Nothing;
            prefs.reminders.fullscreen_audio = AudioMode::Silent;
            prefs.reminders.audio = if case.ends_with("chime") {
                AudioMode::Chime
            } else {
                AudioMode::Voice
            };
            store.save(&prefs)?;
            store.accept_phrase(
                &kavychka_storage::library::Phrase {
                    id: "p4-policy-ready".into(),
                    text: "This is a synthetic reminder for a short audio cancellation test. "
                        .repeat(12),
                    category: "lunch".into(),
                    model: "system".into(),
                    profile: "system".into(),
                    day: Some(clock(0).date.to_string()),
                    audio_path: None,
                    approved: true,
                    deleted: false,
                    rejected: false,
                    used_at: None,
                    created_at: clock(0).utc.to_rfc3339(),
                    followup: false,
                    language: "en".into(),
                    seed: 42,
                    voice_style: String::new(),
                    voice_instruction: String::new(),
                    voice_reference: String::new(),
                },
                None,
            )?;
        }
        std::fs::write(data.join("p4-fixture.owner"), p4_crash::OWNER)?;
        let evidence = output.join(case).join("evidence");
        let process = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(&data)
            .arg("--p4-crash-child")
            .arg(&evidence)
            .arg(case)
            .output()?;
        std::fs::write(output.join(case).join("stderr.log"), &process.stderr)?;
        if !process.status.success() {
            return Err(format!(
                "native service audio {case}: {}",
                String::from_utf8_lossy(&process.stderr)
            )
            .into());
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(evidence.join("result.json"))?)?;
        if result["passed"] != true {
            return Err("native audio child did not pass".into());
        }
        rows.push(result);
        println!("P4 real service audio passed: {case}");
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"cases":rows,"real_service_engine_storage_wasapi":true,
            "restrictions_and_clock_injected":true,"physical_os_transitions_tested":false,
            "physical_output_heard_by_human":false,"models_used":false,"user_library_opened":false
        }))?,
    )?;
    Ok(())
}
pub fn child(
    data: PathBuf,
    output: PathBuf,
    case: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // The caller already checked the disposable owner and installed the
    // observer in this process. Normal application launches never install it.
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
    let result = Rc::new(RefCell::new(None));
    let outcome = result.clone();
    let started = Instant::now();
    let mut phase = 0;
    let mut changed_at = Instant::now();
    let mut quiet_at = Instant::now();
    let exception = case == "policy-dnd-exception";
    let mut cancelled_ms = 0u128;
    let cancellation_time = Rc::new(RefCell::new(0u128));
    let saved_time = cancellation_time.clone();
    let scenario = case.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(10),
        move || {
            let case = scenario.as_str();
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let pass = (|| -> Result<(), String> {
                if started.elapsed() > Duration::from_secs(15) {
                    return Err(format!("audio case {case} timed out at {phase}"));
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string());
                }
                let Some(view) = shell.view.borrow().clone() else {
                    return Ok(());
                };
                let counts = p4_crash::audio_counts();
                match phase {
                    0 if ui.get_ready() => {
                        step(
                            &shell,
                            0,
                            Environment::default(),
                            Some(vec![Event {
                                id: EVENT.into(),
                                category: Category::Lunch,
                                due: clock(0).utc,
                                followup: false,
                            }]),
                        );
                        phase = 1;
                    }
                    1 if counts.0 == 1
                        && view.audio_busy
                        && shell.message.window().is_visible() =>
                    {
                        changed_at = Instant::now();
                        if case == "policy-pause-voice" {
                            // Actual UI action, not an injected paused flag.
                            shell.tray().invoke_pause_minutes(1);
                        } else {
                            let mut environment = Environment::default();
                            match case {
                                "policy-dnd-voice"
                                | "policy-dnd-chime"
                                | "policy-dnd-exception" => {
                                    environment.notifications = NotificationState::Blocked
                                }
                                "policy-lock-voice" => environment.locked = true,
                                "policy-session-voice" => environment.session_unknown = true,
                                "policy-suspend-voice" => environment.suspended = true,
                                "policy-fullscreen-voice" => environment.fullscreen = true,
                                _ => return Err("unknown policy audio case".into()),
                            }
                            step(&shell, 1, environment, None);
                        }
                        phase = 2;
                    }
                    2 => {
                        let applied = if case == "policy-pause-voice" {
                            view.paused
                        } else {
                            view.controlled_time == Some(clock(1).utc.timestamp_millis())
                        };
                        if applied {
                            if exception {
                                if counts != (1, 0) || !view.audio_busy || view.notice.is_none() {
                                    return Err("DND exception stopped live audio".into());
                                }
                                if changed_at.elapsed() >= Duration::from_millis(400) {
                                    shell.send(Command::Silence);
                                    phase = 5;
                                }
                            } else if counts.1 == 1 && !view.audio_busy {
                                cancelled_ms = changed_at.elapsed().as_millis();
                                if cancelled_ms > 2000
                                    || view.notice.is_some()
                                    || shell.message.window().is_visible()
                                {
                                    return Err(
                                        "restriction did not promptly cancel and hide real audio"
                                            .into(),
                                    );
                                }
                                if view.paused {
                                    shell.send(Command::Pause(0));
                                }
                                step(&shell, 2, Environment::default(), None);
                                quiet_at = Instant::now();
                                phase = 3;
                            } else if changed_at.elapsed() > Duration::from_secs(2) {
                                return Err(
                                    "actual WASAPI cancellation did not reach its terminal phase"
                                        .into(),
                                );
                            }
                        }
                    }
                    3 if view.controlled_time == Some(clock(2).utc.timestamp_millis())
                        && quiet_at.elapsed() > Duration::from_millis(400) =>
                    {
                        if counts != (1, 1) || view.audio_busy || view.notice.is_some() {
                            return Err("unblock replayed interrupted audio or message".into());
                        }
                        *saved_time.borrow_mut() = cancelled_ms;
                        *outcome.borrow_mut() = Some(Ok(()));
                        ui.invoke_quit();
                    }
                    5 if counts == (1, 1) && !view.audio_busy => {
                        *outcome.borrow_mut() = Some(Ok(()));
                        ui.invoke_quit();
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = pass {
                *outcome.borrow_mut() = Some(Err(error));
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
    result
        .borrow_mut()
        .take()
        .ok_or("native audio test stopped early")?
        .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
    let store = Sandbox::open(&data)?;
    let journal = store
        .delivery_journal(&format!("{EVENT}#0"))?
        .ok_or("delivery journal missing")?;
    for phase in [
        "committed",
        "shown",
        "audio_requested",
        "audio_preparing",
        "audio_started",
        "audio_cancelled",
    ] {
        if !journal.phases.contains_key(phase) {
            return Err(format!("actual audio phase {phase} was not committed").into());
        }
    }
    if journal.phrase_id.as_deref() != Some("p4-policy-ready")
        || journal.phases.contains_key("audio_completed")
        || journal.phases.contains_key("confirmed")
        || store
            .phrases()?
            .iter()
            .filter(|p| p.used_at.is_some())
            .count()
            != 1
    {
        return Err("cancelled delivery identity or lifecycle changed".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"case":case,"actual_pcm_started":true,"actual_pcm_cancelled":true,
            "cancel_observed_ms":*cancellation_time.borrow(),"journal":journal,
            "dnd_exception_kept_audio_until_explicit_stop":exception,"clock_and_restrictions_injected":true,
            "real_tray_pause_action":case=="policy-pause-voice","physical_os_transition":false
        }))?,
    )?;
    Ok(())
}
