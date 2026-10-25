//! Opt-in owned-window measurements. Not a substitute for the missing 8GB laptop.
use crate::*;
use serde_json::json;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
pub fn run(output: PathBuf, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("Measurements require a release build".into());
    }
    if !output.is_absolute() || output.exists() || seconds < 600 {
        return Err(
            "New absolute evidence folder and at least 600 seconds per mode required".into(),
        );
    }
    std::fs::create_dir_all(&output)?;
    let data = output.join("data");
    {
        let mut s = Sandbox::open(&data)?;
        let mut p = s.load()?;
        p.desktop.setup_complete = true;
        p.desktop.automatic_updates = false;
        p.desktop.run_at_login = false;
        p.generation.auto_daily = false;
        p.generation.text_enabled = false;
        p.generation.tts_model = "system".into();
        p.schedule.work_days.clear();
        p.schedule.exercise_enabled = false;
        p.reminders.audio = AudioMode::Silent;
        s.save(&p)?;
    }
    shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_runtime(data, ui.as_weak());
    wire(&ui, &backend);
    let companion = shell::Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let pet = companion.clone();
    let result = Rc::new(RefCell::new(None));
    let outcome = result.clone();
    let started = Instant::now();
    let evidence = output.clone();
    let mut ready = None;
    let mut baseline = None;
    let mut phase = 0u8;
    let mut phase_start = Instant::now();
    let mut samples = vec![];
    let mut modes = vec![];
    let mut pending_latency = None;
    let mut latencies = vec![];
    let logical = std::thread::available_parallelism()?.get();
    timer.start(slint::TimerMode::Repeated,Duration::from_secs(1),move||{
        let Some(ui)=weak.upgrade()else{return};
        let pass=(||->Result<(),String>{
            let sample=kavychka_platform::process::sample()?;
            if ui.get_failed(){return Err(ui.get_status().to_string())}
            if ready.is_none(){
                if ui.get_ready(){ready=Some(sample.age_millis);phase_start=Instant::now();}
                else if started.elapsed()>Duration::from_secs(30){return Err("UI readiness timeout (mascot may be suppressed by a system policy)".into())}
                return Ok(());
            }
            if phase==0 {
                if phase_start.elapsed()<Duration::from_secs(60){return Ok(())}
                phase=1;phase_start=Instant::now();baseline=Some(sample);
            }
            samples.push(json!({"mode":phase,"elapsed_seconds":phase_start.elapsed().as_secs_f64(),"private_bytes":sample.private_bytes,"cpu_100ns":sample.cpu_100ns,"mascot_visible":pet.pet.window().is_visible()}));
            if phase==2 && pending_latency.is_none(){
                let queued=Instant::now();let weak=ui.as_weak();let sink=std::sync::Arc::new(std::sync::Mutex::new(None));
                let callback=sink.clone();
                slint::invoke_from_event_loop(move||{if let Some(ui)=weak.upgrade(){ui.set_page(if ui.get_page()==7{0}else{7});if let Ok(mut value)=callback.lock(){*value=Some(queued.elapsed().as_secs_f64()*1000.);}}}).map_err(|e|e.to_string())?;
                pending_latency=Some(sink);
            }
            let value=pending_latency.as_ref().and_then(|s|s.lock().ok().and_then(|mut v|v.take()));
            if let Some(value)=value{latencies.push(value);pending_latency=None;}
            if phase_start.elapsed()>=Duration::from_secs(seconds){
                let first=baseline.ok_or("Missing measurement baseline")?;
                let wall=phase_start.elapsed().as_secs_f64();
                let cpu=(sample.cpu_100ns-first.cpu_100ns) as f64/10_000_000./wall/logical as f64*100.;
                let peak=samples.iter().filter(|v|v["mode"]==phase).filter_map(|v|v["private_bytes"].as_u64()).max().unwrap_or(0);
                modes.push(json!({"mode":if phase==1{"mascot_idle"}else{"settings_navigation"},"seconds":wall,"cpu_percent_machine":cpu,"private_bytes_peak":peak,"private_bytes_start":first.private_bytes,"private_bytes_end":sample.private_bytes}));
                if phase==1{phase=2;phase_start=Instant::now();ui.show().map_err(|e|e.to_string())?;baseline=Some(sample);}
                else{
                    latencies.sort_by(f64::total_cmp);let p95=latencies.get((latencies.len()*95/100).min(latencies.len().saturating_sub(1))).copied();
                    let report=json!({"format":1,"release":true,"app_version":app_version(),"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?,"logical_cpu_denominator":logical,"warmup_seconds":60,"readiness_from_process_creation_ms":ready,"modes":modes,"event_loop_navigation_p95_ms":p95,"latency_samples":latencies.len(),"scope":"Current machine, own live shell, no ML; callback navigation latency is not full physical input or latency under generation. Tree process measurements must be supplied separately.","target_laptop_tested":false,"24h_tested":false,"physical_P4_tested":false,"samples":samples});
                    std::fs::write(evidence.join("result.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                    *outcome.borrow_mut()=Some(Ok(()));let _=slint::quit_event_loop();
                }
            }
            Ok(())
        })();
        if let Err(e)=pass{*outcome.borrow_mut()=Some(Err(e));let _=slint::quit_event_loop();}
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    drop(timer);
    drop(companion);
    drop(backend);
    result
        .borrow_mut()
        .take()
        .ok_or("Measurement loop ended early")?
        .map_err(Into::into)
}
