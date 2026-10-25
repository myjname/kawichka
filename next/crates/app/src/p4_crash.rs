//! Explicit, disposable-fixture fault points. In normal runs no trap is installed.
//! A fault exits the whole child process without running Rust destructors.
use crate::*;
use kavychka_core::{
    engine::{Clock, Environment},
    reminders::{Category, Event},
};
use kavychka_platform::audio::{Phase, PhaseObserver};
use slint::ComponentHandle;
use std::{
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
pub(crate) const OWNER: &str = "kavychka-p4-disposable-crash-fixture-v1\n";
const STAGES: &[&str] = &[
    "before_claim_commit",
    "after_claim_commit",
    "after_show",
    "audio_started",
    "audio_completed",
    "before_confirm_commit",
    "after_confirm_commit",
];
struct Trap {
    stage: String,
    marker: PathBuf,
}
static TRAP: OnceLock<Trap> = OnceLock::new();
static AUDIO_STARTED: AtomicU64 = AtomicU64::new(0);
static AUDIO_CANCELLED: AtomicU64 = AtomicU64::new(0);
pub fn audio_counts() -> (u64, u64) {
    (
        AUDIO_STARTED.load(Ordering::Acquire),
        AUDIO_CANCELLED.load(Ordering::Acquire),
    )
}
pub fn hit(stage: &str) {
    if let Some(trap) = TRAP.get()
        && trap.stage == stage
    {
        // The marker contains only this fixture's stage, never phrase text.
        if std::fs::write(&trap.marker, stage).is_err() {
            std::process::exit(88);
        }
        std::process::exit(86);
    }
}
pub fn audio_observer() -> Option<PhaseObserver> {
    TRAP.get().map(|_| {
        std::sync::Arc::new(|phase| match phase {
            Phase::Started => {
                AUDIO_STARTED.fetch_add(1, Ordering::AcqRel);
                hit("audio_started");
            }
            Phase::Completed => hit("audio_completed"),
            Phase::Cancelled => {
                AUDIO_CANCELLED.fetch_add(1, Ordering::AcqRel);
            }
            _ => {}
        }) as PhaseObserver
    })
}
fn clock(seconds: i64) -> Clock {
    let utc = "2026-10-23T09:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .expect("fixture UTC")
        + chrono::Duration::seconds(seconds);
    Clock {
        utc,
        mono: 1_000_000 + seconds as u64 * 1000,
        date: utc.date_naive(),
    }
}
fn event() -> Event {
    Event {
        id: "p4-lunch-event".into(),
        category: Category::Lunch,
        due: clock(0).utc,
        followup: false,
    }
}
pub fn child(
    data: PathBuf,
    output: PathBuf,
    stage: String,
) -> Result<(), Box<dyn std::error::Error>> {
    if !crate::p4_audio::supports(&stage)
        && !STAGES.contains(&stage.as_str())
        && ![
            "recover",
            "recover_again",
            "recovery_before_checkpoint",
            "recovery_after_checkpoint",
        ]
        .contains(&stage.as_str())
    {
        return Err("unknown fault point".into());
    }
    if std::fs::read_to_string(data.join("p4-fixture.owner"))? != OWNER {
        return Err("only an owned P4 fixture may be faulted".into());
    }
    let (expected_pending, already_claimed) = {
        let store = Sandbox::open(&data)?;
        (
            store.load_state()?.pending.len(),
            store
                .phrases()?
                .iter()
                .filter(|r| r.used_at.is_some())
                .count(),
        )
    };
    std::fs::create_dir_all(&output)?;
    TRAP.set(Trap {
        stage: stage.clone(),
        marker: output.join("fault.hit"),
    })
    .map_err(|_| "fault trap already installed")?;
    if crate::p4_audio::supports(&stage) {
        return crate::p4_audio::child(data, output, stage);
    }
    shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let recovering = stage.starts_with("recover");
    let recovery_start = if stage == "recover_again" { 32 } else { 30 };
    let backend = Backend::start_qualification(
        data.clone(),
        ui.as_weak(),
        clock(if recovering { recovery_start } else { 0 }),
    );
    wire(&ui, &backend);
    let companion = shell::Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let shell = companion.clone();
    let outcome = std::rc::Rc::new(std::cell::RefCell::new(None));
    let result = outcome.clone();
    let started = Instant::now();
    let mut phase = 0;
    let mut quiet_since = Instant::now();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let pass = (|| -> Result<(), String> {
                if started.elapsed() > Duration::from_secs(12) {
                    return Err(format!("fault child timed out: {stage}, phase {phase}"));
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string());
                }
                let Some(view) = shell.view.borrow().clone() else {
                    return Ok(());
                };
                match phase {
                    0 if ui.get_ready() => {
                        if recovering {
                            if view.pending != expected_pending
                                || (expected_pending > 0 && view.notice.is_none())
                            {
                                return Err("fresh pending state was not restored quietly".into());
                            }
                            quiet_since = Instant::now();
                            phase = 1;
                        } else {
                            shell.send(Command::ControlledStep {
                                clock: clock(0),
                                environment: Environment::default(),
                                events: Some(vec![event()]),
                                input: vec![],
                            });
                            phase = 1;
                        }
                    }
                    1 if recovering && quiet_since.elapsed() > Duration::from_millis(400) => {
                        if AUDIO_STARTED.load(Ordering::Acquire) != 0 {
                            return Err("recovery replayed audio".into());
                        }
                        // Re-offer the exact event. Already handled work must not consume another row.
                        shell.send(Command::ControlledStep {
                            clock: clock(recovery_start + 1),
                            environment: Environment::default(),
                            events: Some(vec![event()]),
                            input: vec![],
                        });
                        phase = 2;
                    }
                    1 if !recovering
                        && view.notice.is_some()
                        && shell.message.window().is_visible() =>
                    {
                        if stage == "before_confirm_commit" || stage == "after_confirm_commit" {
                            shell.message.invoke_confirmed();
                            phase = 2;
                        }
                    }
                    2 if recovering
                        && view.controlled_time
                            == Some(clock(recovery_start + 1).utc.timestamp_millis()) =>
                    {
                        if AUDIO_STARTED.load(Ordering::Acquire) != 0 {
                            return Err("duplicate event replayed audio after restart".into());
                        }
                        if view.handled != 1 {
                            return Err("recovered event identity changed".into());
                        }
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
    if !recovering {
        return Err("child exited without hitting its fault point".into());
    }
    outcome
        .borrow_mut()
        .take()
        .ok_or("recovery stopped early")?
        .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
    let store = Sandbox::open(&data)?;
    let used = store
        .phrases()?
        .iter()
        .filter(|r| r.used_at.is_some())
        .count();
    if used != 1 || (already_claimed != 0 && used != already_claimed) {
        return Err("restart consumed a phrase twice".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"already_claimed":already_claimed,"used_after_restart":used,"audio_started_after_restart":AUDIO_STARTED.load(Ordering::Acquire),"pending_restored":expected_pending,"controlled_time":true,"real_winit":true,"python_or_models":false}),
        )?,
    )?;
    Ok(())
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&output)?;
    let mut rows = vec![];
    for stage in STAGES {
        let case = output.join(stage);
        let data = case.join("data");
        std::fs::create_dir_all(&case)?;
        {
            let mut store = Sandbox::open(&data)?;
            let mut prefs = store.load()?;
            prefs.desktop.setup_complete = true;
            prefs.desktop.automatic_updates = false;
            prefs.schedule.work_days.clear();
            prefs.generation.auto_daily = false;
            prefs.generation.runtime = false;
            prefs.speech_language = Language::En;
            prefs.reminders.audio = if stage.starts_with("audio_") {
                AudioMode::Chime
            } else {
                AudioMode::Silent
            };
            prefs.reminders.fullscreen_visual = VisualMode::Both;
            store.save(&prefs)?;
            for i in 0..2 {
                store.accept_phrase(
                    &kavychka_storage::library::Phrase {
                        id: format!("p4-ready-{i}"),
                        text: format!("A short synthetic lunch reminder {i}."),
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
        }
        std::fs::write(data.join("p4-fixture.owner"), OWNER)?;
        let process = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(&data)
            .arg("--p4-crash-child")
            .arg(case.join("fault"))
            .arg(stage)
            .output()?;
        std::fs::write(case.join("fault.stderr.log"), &process.stderr)?;
        if process.status.code() != Some(86)
            || std::fs::read_to_string(case.join("fault/fault.hit"))? != *stage
        {
            return Err(format!(
                "fault {stage} did not terminate at the requested boundary: {}",
                String::from_utf8_lossy(&process.stderr)
            )
            .into());
        }
        let (used, pending, journal) = {
            let store = Sandbox::open(&data)?;
            (
                store
                    .phrases()?
                    .iter()
                    .filter(|r| r.used_at.is_some())
                    .count(),
                store.load_state()?.pending.len(),
                store.delivery_journal("p4-lunch-event#0")?,
            )
        };
        let expected_used = usize::from(*stage != "before_claim_commit");
        let expected_pending =
            usize::from(*stage != "before_claim_commit" && *stage != "after_confirm_commit");
        if used != expected_used || pending != expected_pending {
            return Err(format!(
                "fault {stage} persisted wrong claim/confirmation: {used}/{pending}"
            )
            .into());
        }
        if *stage == "before_claim_commit" {
            if journal.is_some() {
                return Err("uncommitted claim left a delivery journal".into());
            }
        } else {
            let journal = journal.as_ref().ok_or("committed claim lost its journal")?;
            if !journal.phases.contains_key("committed")
                || journal.event_ids != ["p4-lunch-event"]
                || journal.phrase_id.as_deref() != Some("p4-ready-1")
                || journal.phases.contains_key("confirmed") != (*stage == "after_confirm_commit")
                || (stage.starts_with("audio_") && !journal.phases.contains_key("audio_requested"))
            {
                return Err(format!("wrong delivery journal at {stage}: {journal:?}").into());
            }
        }
        if *stage == "after_show" {
            for boundary in ["recovery_before_checkpoint", "recovery_after_checkpoint"] {
                let repeated_fault = std::process::Command::new(std::env::current_exe()?)
                    .arg("--data-dir")
                    .arg(&data)
                    .arg("--p4-crash-child")
                    .arg(case.join(boundary))
                    .arg(boundary)
                    .output()?;
                std::fs::write(
                    case.join(format!("{boundary}.stderr.log")),
                    &repeated_fault.stderr,
                )?;
                if repeated_fault.status.code() != Some(86)
                    || std::fs::read_to_string(case.join(boundary).join("fault.hit"))? != boundary
                {
                    return Err(format!("recovery fault did not hit {boundary}").into());
                }
                let store = Sandbox::open(&data)?;
                if store
                    .phrases()?
                    .iter()
                    .filter(|r| r.used_at.is_some())
                    .count()
                    != used
                    || store.load_state()?.pending.len() != pending
                {
                    return Err("a fault during recovery changed claim/confirmation".into());
                }
            }
        }
        let recovery = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(&data)
            .arg("--p4-crash-child")
            .arg(case.join("recovery"))
            .arg("recover")
            .output()?;
        std::fs::write(case.join("recovery.stderr.log"), &recovery.stderr)?;
        if !recovery.status.success() {
            return Err(format!(
                "recovery {stage}: {}",
                String::from_utf8_lossy(&recovery.stderr)
            )
            .into());
        }
        let again = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(&data)
            .arg("--p4-crash-child")
            .arg(case.join("second-recovery"))
            .arg("recover_again")
            .output()?;
        std::fs::write(case.join("second-recovery.stderr.log"), &again.stderr)?;
        if !again.status.success() {
            return Err(format!(
                "second recovery failed at {stage}: {}",
                String::from_utf8_lossy(&again.stderr)
            )
            .into());
        }
        rows.push(serde_json::json!({"stage":stage,"exit_code":86,"used_at_fault":used,"pending_at_fault":pending,"journal_at_fault":journal,"quiet_restart":true,"no_second_claim":true}));
        println!("P4 crash boundary passed: {stage}");
    }
    crate::p4_audio::run(output.join("service-audio"))?;
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"boundaries":rows,"repeated_restart_each_boundary":true,"crash_during_recovery_before_and_after_checkpoint":true,"whole_process_exit_without_destructors":true,"actual_wasapi_started_and_completed_boundaries":true,"actual_message_window":true,"controlled_clock":true,"physical_audio_heard_not_provable":true,"user_library_opened":false,"models_used":false}),
        )?,
    )?;
    Ok(())
}
