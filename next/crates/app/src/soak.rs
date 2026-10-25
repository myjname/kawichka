//! Opt-in unattended native qualification. Run on an owned non-input desktop.
//! Silent synthetic data; streaming evidence and bounded in-memory observations.
use crate::{AppWindow, AudioMode, Preferences, Sandbox, service::Backend};
use serde_json::json;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    io::Write,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use kavychka_platform::time::awake_ms;

pub fn run(output: PathBuf, seconds: u64, live: bool) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions)
        || !output.is_absolute()
        || output.exists()
        || !matches!(seconds, 12 | 1200 | 86400)
    {
        return Err("Release build, new absolute evidence and explicit 12/1200/86400-second QA mode required".into());
    }
    std::fs::create_dir_all(&output)?;
    let data = output.join("data");
    let zone = {
        let mut store = Sandbox::open(&data)?;
        let mut prefs = store.load()?;
        prefs.desktop.setup_complete = true;
        prefs.desktop.automatic_updates = false;
        prefs.desktop.run_at_login = false;
        prefs.generation.auto_daily = false;
        prefs.generation.text_enabled = false;
        prefs.generation.tts_model = "system".into();
        prefs.schedule.work_days.clear();
        prefs.schedule.exercise_enabled = false;
        prefs.reminders.audio = AudioMode::Silent;
        let zone = prefs.desktop.time_zone.clone();
        store.save(&prefs)?;
        zone
    };
    let scheduling = kavychka_platform::ui_scheduling::UiScheduling::prefer_responsive()
        .ok()
        .flatten();
    let policy=scheduling.as_ref().map(|s|json!({"preferred":s.preferred,"group":s.group,"selected_cpu_count":s.selected_count,"hard_affinity_changed":false}));
    let exe_hash =
        kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e| e.to_string())?;
    crate::shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = if live {
        Backend::start_runtime(data.clone(), ui.as_weak())
    } else {
        Backend::start_qualification(
            data.clone(),
            ui.as_weak(),
            kavychka_platform::time::now_in(&zone).map_err(|e| e.to_string())?,
        )
    };
    let controller = backend.sender.clone();
    crate::wire(&ui, &backend);
    let companion = crate::shell::Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let result = Rc::new(RefCell::new(
        None::<Result<(String, serde_json::Value), String>>,
    ));
    let outcome = result.clone();
    let weak = ui.as_weak();
    let pet = companion.clone();
    let timer = slint::Timer::default();
    let started = Instant::now();
    let start_awake = awake_ms()?;
    let evidence = output.clone();
    let warmup = if seconds == 12 { 2 } else { 60 };
    let mut ready = None;
    let mut measured = None;
    let mut mode = 0;
    let mut peak = 0u64;
    let mut baseline = None;
    let mut mode_samples = 0u64;
    let mut pet_visible = 0u64;
    let mut settings_visible = 0u64;
    let mut maximum_gap = 0u64;
    let mut prior_tick = None;
    let mut completed_modes = vec![];
    let mut last_progress = 0;
    let mut last_save = 0;
    let mut save_serial = 0u64;
    let mut saved = 0u64;
    let mut pending_save = None::<(String, u64)>;
    let mut last_marker = String::new();
    let mut last_restore = 0;
    let restore_result = Rc::new(RefCell::new(None::<Result<(), String>>));
    let restore_sink = restore_result.clone();
    let mut restores = 0u64;
    let mut test_notice_requested = false;
    let mut test_notice_confirmed = false;
    let mut stream = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("samples.jsonl"))?,
    );
    timer.start(slint::TimerMode::Repeated,Duration::from_secs(1),move||{
        let Some(ui)=weak.upgrade()else{return};
        let step=(||->Result<(),String>{
            let awake=awake_ms()?;
            if !live {
                controller.try_send(crate::service::Command::ControlledStep {
                    clock:kavychka_platform::time::now_in(&zone).map_err(|e|e.to_string())?,
                    environment:Default::default(),events:None,input:vec![],
                }).map_err(|e|e.to_string())?;
            }
            let process=kavychka_platform::process::sample()?;
            if ui.get_failed(){return Err(ui.get_status().to_string())}
            if ready.is_none(){
                if ui.get_ready(){ready=Some(awake);}
                else if awake.saturating_sub(start_awake)>30_000{return Err("Native readiness timeout".into())}
                return Ok(());
            }
            if measured.is_none(){
                if awake.saturating_sub(ready.unwrap())<warmup*1000{return Ok(())}
                measured=Some(awake);baseline=Some(process);mode=1;
            }
            let elapsed=awake.saturating_sub(measured.unwrap());
            if seconds==12 && elapsed>=1000 && !test_notice_requested {
                ui.invoke_stretch_now();test_notice_requested=true;
            }
            if seconds==12 && pet.message.window().is_visible()
                && pet.view.borrow().as_ref().is_some_and(|v|v.notice.is_some()) {
                pet.message.invoke_confirmed();test_notice_confirmed=true;
            }
            if let Some(prior)=prior_tick{maximum_gap=maximum_gap.max(awake.saturating_sub(prior));}
            prior_tick=Some(awake);
            if let Some(error)=restore_sink.borrow_mut().take(){error?;restores+=1;}
            if let Some((marker,since))=&pending_save{
                if !ui.get_busy() && !ui.get_dirty(){
                    let prefs:Preferences=serde_json::from_str(ui.get_loaded_prefs().as_str()).map_err(|e|e.to_string())?;
                    if prefs.user_name!=*marker{return Err("Native save acknowledged with wrong persisted marker".into())}
                    last_marker=marker.clone();saved+=1;pending_save=None;
                }else if awake.saturating_sub(*since)>10_000{return Err("Native save acknowledgement timeout".into())}
            }
            if mode==1 && elapsed>=seconds*500{
                completed_modes.push(json!({"mode":"mascot_requested","samples":mode_samples,"private_bytes_peak":peak,"pet_visible_samples":pet_visible,"settings_visible_samples":settings_visible}));
                mode=2;peak=0;mode_samples=0;pet_visible=0;settings_visible=0;
                ui.show().map_err(|e|e.to_string())?;
            }
            mode_samples+=1;peak=peak.max(process.private_bytes);
            pet_visible+=u64::from(pet.pet.window().is_visible());
            settings_visible+=u64::from(ui.window().is_visible());
            if mode==2{
                ui.set_page(((elapsed/5000)%8) as i32);
                let save_every=if seconds==12{2000}else{60_000};
                if pending_save.is_none() && elapsed.saturating_sub(last_save)>=save_every && elapsed+2000<seconds*1000{
                    save_serial+=1;let marker=format!("SOAK-{save_serial}");
                    ui.set_user_name(marker.as_str().into());ui.set_theme_index((save_serial%3) as i32);
                    ui.set_language_index((save_serial%2) as i32);
                    ui.invoke_edited();ui.invoke_save();
                    pending_save=Some((marker,awake));last_save=elapsed;
                }
                let restore_every=if seconds==12{2000}else{300_000};
                if elapsed.saturating_sub(last_restore)>=restore_every && elapsed+1000<seconds*1000{
                    last_restore=elapsed;ui.window().set_minimized(true);
                    let target=ui.as_weak();let sink=restore_sink.clone();let image=evidence.join("last-restored.png");
                    slint::Timer::single_shot(Duration::from_millis(200),move||{
                        if let Some(ui)=target.upgrade(){
                            ui.window().set_minimized(false);ui.window().request_redraw();
                            let result=(||->Result<(),String>{
                                let pixels=ui.window().take_snapshot().map_err(|e|e.to_string())?;
                                if pixels.width()<100 || pixels.height()<100 || pixels.as_slice().iter().step_by(997).any(|p|p.a!=255){return Err("Restored native client snapshot is transparent or empty".into())}
                                image::save_buffer(&image,pixels.as_bytes(),pixels.width(),pixels.height(),image::ColorType::Rgba8).map_err(|e|e.to_string())?;
                                Ok(())
                            })();*sink.borrow_mut()=Some(result);
                        }
                    });
                }
            }
            let sample=json!({"awake_ms":elapsed,"wall_seconds":started.elapsed().as_secs_f64(),"mode":mode,"private_bytes":process.private_bytes,"cpu_100ns":process.cpu_100ns,"handles":process.handle_count,"gdi_objects":process.gdi_objects,"user_objects":process.user_objects,"pet_visible":pet.pet.window().is_visible(),"settings_visible":ui.window().is_visible()});
            serde_json::to_writer(&mut stream,&sample).map_err(|e|e.to_string())?;writeln!(stream).map_err(|e|e.to_string())?;
            if elapsed.saturating_sub(last_progress)>=if seconds==12{1000}else{30_000}{
                stream.flush().map_err(|e|e.to_string())?;
                kavychka_maintenance::atomic(&evidence.join("progress.json"),&json!({"format":1,"state":"running","app_sha256":exe_hash,"target_awake_seconds":seconds,"observed_awake_seconds":elapsed as f64/1000.,"wall_seconds":started.elapsed().as_secs_f64(),"mode":mode,"saved":saved,"restores":restores,"private_bytes":process.private_bytes,"max_awake_tick_gap_ms":maximum_gap,"native_windows":true,"desktop_input_injected":false})).map_err(|e|e.to_string())?;
                last_progress=elapsed;
            }
            if elapsed>=seconds*1000 && pending_save.is_none(){
                stream.flush().map_err(|e|e.to_string())?;
                completed_modes.push(json!({"mode":"settings_navigation","samples":mode_samples,"private_bytes_peak":peak,"pet_visible_samples":pet_visible,"settings_visible_samples":settings_visible}));
                if saved==0 || restores==0{return Err("Qualification did not exercise save and native restore".into())}
                if pet.view.borrow().is_none(){return Err("Native shell never consumed a service view (tray failure must not block presentation)".into())}
                if seconds==12 && !test_notice_confirmed{return Err("Silent native reminder was not presented and confirmed without a tray".into())}
                let first=baseline.ok_or("Missing native baseline")?;
                let cpu=(process.cpu_100ns-first.cpu_100ns) as f64/10_000_000./(elapsed as f64/1000.)/std::thread::available_parallelism().map_err(|e|e.to_string())?.get() as f64*100.;
                let report=json!({"format":1,"passed":true,"app_version":crate::app_version(),"app_sha256":exe_hash,"native_windows":true,"target_awake_seconds":seconds,"observed_awake_seconds":elapsed as f64/1000.,"wall_seconds":started.elapsed().as_secs_f64(),"24h_awake_observed":seconds>=86400,"uninterrupted_ui_tick_budget_passed":maximum_gap<=5000,"maximum_awake_tick_gap_ms":maximum_gap,"saved_settings_roundtrips":saved,"native_minimize_restore_snapshots":restores,"cpu_percent_machine":cpu,"modes":completed_modes,"scheduling":policy,"physical_input_tested":false,"active_desktop_compositor_tested":false,"screen_reader_tested":false,"target_laptop_tested":false,"audio_played":false,"models_loaded":false,"scope":"Native Winit/software AppWindow + real service and Shell on an owned non-input desktop; no Explorer tray qualification. Wall sleep is excluded using QueryUnbiasedInterruptTime. Raw observations streamed, not retained in RAM. A 12-second self-test is harness validation only; 1200 seconds is a short qualification, not 24h."});
                let mut report=report;
                report["platform_environment_injected"]=json!(!live);
                report["live_platform_monitors"]=json!(live);
                report["scope"]=json!(if live {"Native Winit/software AppWindow, real live platform adapters, service and Shell on a non-input desktop; actual private-desktop notification restrictions apply. No Explorer tray qualification, physical input, active compositor or screen reader check. Sleep excluded using QueryUnbiasedInterruptTime."} else {"Native Winit/software AppWindow, service and Shell on a non-input desktop with explicitly allowed injected platform environment and real advancing clock. No live input/session/DND adapter qualification. No Explorer tray, physical input, active compositor or screen reader check. Sleep excluded using QueryUnbiasedInterruptTime."});
                report["self_test_reminder_presented_and_confirmed"]=json!(if seconds==12{Some(test_notice_confirmed)}else{None});
                *outcome.borrow_mut()=Some(Ok((last_marker.clone(),report)));let _=slint::quit_event_loop();
            }
            Ok(())
        })();
        if let Err(error)=step{
            let _=kavychka_maintenance::atomic(&evidence.join("progress.json"),&json!({"state":"failed","error":error}));
            *outcome.borrow_mut()=Some(Err(error));let _=slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    drop(timer);
    drop(companion);
    drop(backend);
    let (marker, report) = result
        .borrow_mut()
        .take()
        .ok_or("Native qualification ended early")??;
    let store = Sandbox::open(&data)?;
    if store.load()?.user_name != marker {
        kavychka_maintenance::atomic(
            &output.join("progress.json"),
            &json!({"state":"failed","error":"Final persisted settings mismatch"}),
        )
        .map_err(|e| e.to_string())?;
        return Err("Persisted final soak settings do not match acknowledged save".into());
    }
    kavychka_maintenance::atomic(&output.join("result.json"), &report)
        .map_err(|e| e.to_string())?;
    kavychka_maintenance::atomic(
        &output.join("progress.json"),
        &json!({"state":"complete","observed_awake_seconds":report["observed_awake_seconds"]}),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn awake_clock_is_available_and_monotonic() {
        let first = super::awake_ms().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(super::awake_ms().unwrap() >= first);
    }
}
