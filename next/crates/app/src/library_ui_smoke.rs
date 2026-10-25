//! Native UI verification against an explicitly prepared private working copy.
use crate::{
    AppWindow,
    models_library::View,
    service::Backend,
    shell::{self, Shell},
    wire,
};
use kavychka_storage::sandbox::Sandbox;
use serde_json::json;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
fn snapshot(ui: &AppWindow, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let p = ui.window().take_snapshot()?;
    image::save_buffer(
        path,
        p.as_bytes(),
        p.width(),
        p.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
pub fn run(data: PathBuf, evidence: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&evidence)?;
    let mut store = Sandbox::open(&data)?;
    let mut prefs = store.load()?;
    prefs.generation.auto_daily = false;
    prefs.schedule.work_days.clear();
    prefs.reminders.audio = kavychka_core::preferences::AudioMode::Silent;
    prefs.generation.tts_model = "tts-cosyvoice3".into();
    store.save(&prefs)?;
    let row = store
        .phrases()?
        .into_iter()
        .find(|r| !r.deleted && !r.rejected && r.used_at.is_none() && r.audio_path.is_some())
        .ok_or("prepare a generated unused recording first")?;
    let text = row.text.clone();
    let id = row.id.clone();
    let old_audio = row.audio_path.clone();
    let old_count = store.phrases()?.len();
    drop(store);
    shell::select_backend(false)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_runtime(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    let companion = Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let outcome = Rc::new(RefCell::new(None::<Result<(), String>>));
    let result = outcome.clone();
    let phase = Rc::new(std::cell::Cell::new(0));
    let stage = phase.clone();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let own = companion.clone();
    let output = evidence.clone();
    let row_id = id.clone();
    let started = Instant::now();
    let mut phase_started = Instant::now();
    let mut saw_busy = false;
    let jobs_dir = data.join("jobs");
    let mut system_jobs = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let check = (|| -> Result<(), Box<dyn std::error::Error>> {
                if started.elapsed() > Duration::from_secs(1200) {
                    return Err(format!("library UI timed out at phase {}", stage.get()).into());
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string().into());
                }
                if !ui.get_ready() || ui.get_library_json().is_empty() {
                    return Ok(());
                }
                let view: View = serde_json::from_str(ui.get_library_json().as_str())?;
                let current = view
                    .phrases
                    .iter()
                    .find(|r| r.id == row_id)
                    .ok_or("row disappeared")?;
                match stage.get() {
                    0 => {
                        if view
                            .profiles
                            .iter()
                            .any(|(_, name)| name.starts_with("Недоступен"))
                        {
                            return Err("a copied voice preset was reported missing".into());
                        }
                        if view.imported == 0
                            || !view
                                .models
                                .iter()
                                .any(|m| m.id == "tts-cosyvoice3" && m.installed && m.environment)
                        {
                            return Err(
                                "imported library or installed model missing from UI".into()
                            );
                        }
                        ui.set_page(2);
                        snapshot(&ui, &output.join("library-private.png"))?;
                        ui.invoke_play_phrase(row_id.as_str().into());
                        phase_started = Instant::now();
                        stage.set(1);
                    }
                    1 => {
                        if own.view.borrow().as_ref().is_some_and(|v| v.audio_busy) {
                            saw_busy = true;
                        }
                        if saw_busy && own.view.borrow().as_ref().is_some_and(|v| !v.audio_busy) {
                            if let Some(v) = own.view.borrow().as_ref()
                                && !v.status.starts_with("Напоминания включены")
                                && !v.status.starts_with("Reminders enabled")
                            {
                                return Err(format!("preview audio failed: {}", v.status).into());
                            }
                            ui.invoke_select_phrase(row_id.as_str().into(), false);
                            stage.set(2);
                        } else if phase_started.elapsed() > Duration::from_secs(120) {
                            return Err("recording preview did not complete".into());
                        }
                    }
                    2 if current.rejected => {
                        ui.invoke_select_phrase(row_id.as_str().into(), true);
                        stage.set(3);
                    }
                    3 if current.approved && !current.rejected => {
                        ui.invoke_generate_batch();
                        stage.set(20);
                    }
                    20 if view.running => {
                        ui.invoke_cancel_generation();
                        stage.set(21);
                    }
                    21 if !view.running => {
                        ui.set_page(3);
                        snapshot(&ui, &output.join("models-private.png"))?;
                        ui.invoke_regenerate_speech(row_id.as_str().into());
                        phase_started = Instant::now();
                        stage.set(4);
                    }
                    4 if view.running => stage.set(5),
                    5 if !view.running => {
                        if current.audio_path == old_audio {
                            return Err(
                                format!("voice regeneration failed: {}", view.status).into()
                            );
                        }
                        if current.text != text {
                            return Err("voice regeneration changed text".into());
                        }
                        ui.invoke_export_library();
                        stage.set(6);
                    }
                    6 if view.running => stage.set(7),
                    7 if !view.running => {
                        if !view.status.starts_with("ZIP:") {
                            return Err(format!("export failed: {}", view.status).into());
                        }
                        ui.set_page(4);
                        snapshot(&ui, &output.join("character-private.png"))?;
                        ui.set_delivery_name("QA delivery".into());
                        ui.set_delivery_instruction("Speak evenly with a subtle smile.".into());
                        ui.invoke_add_delivery();
                        if !ui.get_dirty() {
                            return Err("custom delivery was not added to draft".into());
                        }
                        ui.invoke_discard();
                        stage.set(8);
                    }
                    8 if !ui.get_dirty() && !ui.get_busy() => {
                        ui.set_page(3);
                        ui.set_voice_model_index(0);
                        ui.invoke_edited();
                        ui.invoke_save();
                        stage.set(9);
                    }
                    9 if !ui.get_dirty() && !ui.get_busy() => {
                        system_jobs = std::fs::read_dir(&jobs_dir)?.count();
                        ui.invoke_regenerate_speech(row_id.as_str().into());
                        stage.set(10);
                    }
                    10 if current.model == "system" => {
                        if current.audio_path.is_some()
                            || std::fs::read_dir(&jobs_dir)?.count() != system_jobs
                        {
                            return Err("system voice unexpectedly used inference".into());
                        }
                        saw_busy = false;
                        phase_started = Instant::now();
                        ui.invoke_play_phrase(row_id.as_str().into());
                        stage.set(11);
                    }
                    11 => {
                        if own.view.borrow().as_ref().is_some_and(|v| v.audio_busy) {
                            saw_busy = true;
                        }
                        if phase_started.elapsed() > Duration::from_secs(120) {
                            return Err("system voice preview did not finish".into());
                        }
                        if saw_busy && own.view.borrow().as_ref().is_some_and(|v| !v.audio_busy) {
                            if let Some(v) = own.view.borrow().as_ref()
                                && !v.status.starts_with("Напоминания включены")
                                && !v.status.starts_with("Reminders enabled")
                            {
                                return Err(format!("system preview failed: {}", v.status).into());
                            }
                            *result.borrow_mut() = Some(Ok(()));
                            ui.invoke_quit();
                        }
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
        .ok_or("UI loop ended without a result")?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let store = Sandbox::open(&data)?;
    let current = store.phrase(&id)?;
    if current.used_at.is_some() {
        return Err("preview consumed a recording".into());
    }
    if store.phrases()?.len() != old_count {
        return Err("voice-only action created or lost a phrase".into());
    }
    let archives = std::fs::read_dir(data.join("exports"))?.count();
    if archives == 0 {
        return Err("ZIP export is missing".into());
    }
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            // Success is reachable only after both Busy -> idle checks above.
            // The timer closure owns its copy of saw_busy; the outer value is stale.
            &json!({"passed":true,"real_windows":true,"imported_library_rendered":true,"models_recognized":true,"recording_preview_started_and_finished":true,"system_voice_preview_started_and_finished":true,"system_voice_used_no_inference_job":true,"generation_cancelled_via_ui":true,"preview_did_not_claim":true,"exclude_keep_persisted":true,"voice_only_regenerated_via_ui":true,"text_and_id_preserved":true,"zip_export_created":true,"custom_delivery_draft_discarded":true,"explicit_exit_and_reopen":true,"live_store_opened_for_writing":false,"screenshots_are_private":true}),
        )?,
    )?;
    println!(
        "Native library/model UI check passed: {}",
        evidence.display()
    );
    Ok(())
}
