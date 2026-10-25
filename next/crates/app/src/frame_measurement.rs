//! Opt-in release qualification. Real AppWindow/service/ML, rendered in memory;
//! no native windows, audio output, desktop input, or live SQLite access.
use crate::{AppWindow, Sandbox, service::Backend};
use serde_json::{Value, json};
use slint::{
    ComponentHandle,
    platform::{EventLoopProxy, Platform, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
type Events = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;
struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    events: Events,
}
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
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.events.clone())))
    }
}
struct Producer {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn prepare(output: &Path, descriptor: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let config: Value = serde_json::from_slice(&std::fs::read(descriptor)?)?;
    let resources = PathBuf::from(config["resources"].as_str().ok_or("Missing resources")?);
    let source = PathBuf::from(config["preset_source"].as_str().ok_or("Missing presets")?);
    if !resources.is_absolute() || !resources.is_dir() || !source.is_absolute() {
        return Err("Explicit existing resources and preset source required".into());
    }
    let mut prefs: crate::Preferences = serde_json::from_value(config["preferences"].clone())?;
    if prefs.generation.tts_model == "system" || !prefs.generation.text_enabled {
        return Err("Measurement requires real local text and speech models".into());
    }
    let mut store = Sandbox::open(&output.join("data"))?;
    if !prefs.generation.voice_profile.is_empty() {
        let id = prefs.generation.voice_profile.clone();
        let preset = kavychka_storage::presets::read(&source, &id)?;
        prefs.generation.voice_profile = kavychka_storage::presets::import_prepared(
            &store.root,
            &preset,
            Some(&kavychka_storage::presets::folder(&source, &id)?),
        )?;
    }
    prefs.desktop.setup_complete = true;
    prefs.desktop.automatic_updates = false;
    prefs.desktop.run_at_login = false;
    prefs.generation.auto_daily = false;
    prefs.generation.runtime = false;
    prefs.generation.ac_only = false; // synthetic environment; no power inference
    prefs.generation.batch_count = 3;
    prefs.generation.categories = vec!["break".into(), "exercise".into(), "lunch".into()];
    prefs.schedule.work_days.clear();
    prefs.schedule.exercise_enabled = false;
    prefs.reminders.audio = crate::AudioMode::Silent;
    store.save(&prefs)?;
    Ok(resources)
}
fn stats(samples: &[Value], active: bool) -> Value {
    let mut values: Vec<_> = samples
        .iter()
        .filter(|s| s["ml_running"] == active)
        .filter_map(|s| s["queue_to_frame_ms"].as_f64())
        .collect();
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return json!({"count":0});
    }
    // Nearest-rank percentile; no discarded outliers or warmup samples here.
    let rank = (values.len() * 95).div_ceil(100) - 1;
    json!({"count":values.len(),"p95_ms":values[rank],"max_ms":values.last(),
        "over_100_ms":values.iter().filter(|v| **v > 100.).count()})
}
pub fn run(output: PathBuf, descriptor: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("Frame measurements require a release build".into());
    }
    if !output.is_absolute() || output.exists() || !descriptor.is_absolute() {
        return Err("New absolute evidence folder and private descriptor required".into());
    }
    std::fs::create_dir_all(&output)?;
    let resources = prepare(&output, &descriptor)?;
    let scheduling = kavychka_platform::ui_scheduling::UiScheduling::prefer_responsive()
        .ok()
        .flatten();
    let window = MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::NewBuffer,
    );
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(output.join("data").to_string_lossy().as_ref().into());
    let backend = Backend::start_measurement(output.join("data"), resources, ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?; // MinimalSoftwareWindow only; never creates an OS window.
    window.set_size(slint::PhysicalSize::new(1100, 780));
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 780);
    let mut pending = None::<Instant>;
    let stop = Arc::new(AtomicBool::new(false));
    let (sent, received) = mpsc::sync_channel::<(Instant, Duration, Duration)>(1);
    let (ack, ack_rx) = mpsc::sync_channel::<()>(1);
    let mut ack_receiver = Some(ack_rx);
    let measure_ui = ui.as_weak();
    let mut producer = None::<Producer>;
    let started = Instant::now();
    let unix_started_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    let mut ready = None;
    let mut phase = "warmup";
    let mut phase_started = Instant::now();
    let mut phase_samples = 0;
    let mut saw_running = false;
    let mut cancel_ms = None;
    let mut completed_rows = 0;
    let mut samples = vec![];
    let mut memory = vec![];
    let mut last_memory = Instant::now();
    let mut baseline = None;
    let mut statuses = std::collections::BTreeSet::new();
    let mut probe = crate::frame_probe::Probe::new(started, output.join("watchdog.json"))?;
    loop {
        probe.begin(1);
        loop {
            let event = events.lock().unwrap().pop_front();
            match event {
                Some(event) => event(),
                None => break,
            }
        }
        slint::platform::update_timers_and_animations();
        probe.end();
        if ui.get_failed() {
            return Err(ui.get_status().to_string().into());
        }
        if started.elapsed() > Duration::from_secs(900) {
            return Err("Frame/ML measurement timed out".into());
        }
        if ready.is_none() && started.elapsed() > Duration::from_secs(30) {
            return Err("Headless UI readiness timed out".into());
        }
        if ready.is_none() && ui.get_ready() && !ui.get_library_json().is_empty() {
            ready = Some(started.elapsed().as_millis());
            phase_started = Instant::now();
        }
        if ready.is_some()
            && phase == "warmup"
            && phase_started.elapsed() >= Duration::from_secs(60)
        {
            baseline = Some(kavychka_platform::process::sample()?);
            let thread_stop = stop.clone();
            let weak = measure_ui.clone();
            let sent = sent.clone();
            let ack_rx = ack_receiver.take().ok_or("Producer already started")?;
            producer = Some(Producer {
                stop: stop.clone(),
                join: Some(std::thread::spawn(move || {
                    let mut page_index = 0;
                    while !thread_stop.load(Ordering::Acquire) {
                        let queued = Instant::now();
                        let ui = weak.clone();
                        let sent = sent.clone();
                        let pages = [0, 2, 3, 5, 7];
                        let page = pages[page_index % pages.len()];
                        page_index += 1;
                        if slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui.upgrade() {
                                let dispatch = queued.elapsed();
                                ui.set_page(page);
                                let _ = sent.try_send((queued, dispatch, queued.elapsed()));
                            }
                        })
                        .is_err()
                        {
                            break;
                        }
                        loop {
                            match ack_rx.recv_timeout(Duration::from_millis(50)) {
                                Ok(()) => break,
                                Err(mpsc::RecvTimeoutError::Timeout)
                                    if !thread_stop.load(Ordering::Acquire) => {}
                                _ => return,
                            }
                        }
                        std::thread::sleep(
                            Duration::from_millis(50).saturating_sub(queued.elapsed()),
                        );
                    }
                })),
            });
            phase = "idle";
            phase_started = Instant::now();
        }
        let mut navigation = None;
        if let Ok((queued, dispatch, callback)) = received.try_recv() {
            if pending.is_some() {
                return Err("Measurement action was coalesced".into());
            }
            pending = Some(queued);
            navigation = Some((dispatch, callback));
            window.request_redraw();
        }
        let render_started = Instant::now();
        let processor_before = kavychka_platform::ui_scheduling::current_processor();
        let cpu_before = probe.cpu()?;
        probe.begin(2);
        let mut rendered = false;
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), 1100);
            rendered = true;
        });
        probe.end();
        let cpu_after = probe.cpu()?;
        let processor_after = kavychka_platform::ui_scheduling::current_processor();
        if rendered && let Some(queued) = pending.take() {
            samples.push(
                json!({"phase":phase,"elapsed_seconds":started.elapsed().as_secs_f64(),
                "page":ui.get_page(),"ml_running":ui.get_generation_running(),
                "processor_before_render":processor_before,"processor_after_render":processor_after,
                "queue_to_frame_ms":queued.elapsed().as_secs_f64()*1000.,
                "dispatch_ms":navigation.map(|v|v.0.as_secs_f64()*1000.),
                "page_set_ms":navigation.map(|v|(v.1-v.0).as_secs_f64()*1000.),
                "render_cpu_ms":cpu_after.saturating_sub(cpu_before) as f64/10_000.,
                "render_ms":render_started.elapsed().as_secs_f64()*1000.}),
            );
            phase_samples += 1;
            ack.send(())?;
        }
        if last_memory.elapsed() >= Duration::from_secs(1) {
            let sample = kavychka_platform::process::sample()?;
            memory.push(json!({"elapsed_seconds":started.elapsed().as_secs_f64(),
                "phase":phase,"private_bytes":sample.private_bytes,"cpu_100ns":sample.cpu_100ns,
                "page_faults":sample.page_faults,"read_bytes":sample.read_bytes,
                "available_physical_bytes":sample.available_physical_bytes}));
            last_memory = Instant::now();
        }
        statuses.insert(ui.get_library_status().to_string());
        match phase {
            "idle" if phase_samples >= 100 => {
                ui.invoke_generate_batch();
                phase = "generate";
                phase_started = Instant::now();
                phase_samples = 0;
            }
            "generate" => {
                saw_running |= ui.get_generation_running();
                if saw_running && !ui.get_generation_running() {
                    let view: crate::models_library::View =
                        serde_json::from_str(ui.get_library_json().as_str())?;
                    completed_rows = view
                        .phrases
                        .iter()
                        .filter(|p| p.audio_path.is_some())
                        .count();
                    if completed_rows != 3 {
                        return Err(format!(
                            "Expected 3 generated recordings, got {completed_rows}; {}",
                            view.status
                        )
                        .into());
                    }
                    if stats(&samples, true)["count"].as_u64().unwrap_or(0) < 300 {
                        return Err("Fewer than 300 active-generation frame samples".into());
                    }
                    ui.invoke_generate_batch();
                    phase = "start_cancel_job";
                    phase_started = Instant::now();
                    saw_running = false;
                }
            }
            "start_cancel_job" => {
                saw_running |= ui.get_generation_running();
                if saw_running && phase_started.elapsed() >= Duration::from_secs(5) {
                    ui.invoke_cancel_generation();
                    phase = "cancel";
                    phase_started = Instant::now();
                }
            }
            "cancel" => {
                if !ui.get_generation_running() {
                    cancel_ms = Some(phase_started.elapsed().as_millis());
                    phase = "after_cancel";
                    phase_samples = 0;
                } else if phase_started.elapsed() > Duration::from_secs(8) {
                    return Err("Model cancellation exceeded 8 seconds".into());
                }
            }
            "after_cancel" if phase_samples >= 100 => break,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(producer); // stop producer before dropping event/backend state
    image::save_buffer(
        output.join("last-frame.png"),
        pixels.as_bytes(),
        1100,
        780,
        image::ColorType::Rgb8,
    )?;
    drop(backend);
    let store = Sandbox::open(&output.join("data"))?;
    let rows = store.phrases()?;
    let prefs = store.load()?;
    let mut decoded = 0;
    for row in rows.iter().filter(|r| r.audio_path.is_some()) {
        let pcm = kavychka_platform::audio::decode_recording(
            &store.audio_file(row)?.ok_or("Recording disappeared")?,
            &|| false,
        )?;
        if pcm.len() <= 8000 {
            return Err("Generated recording is too short".into());
        }
        decoded += 1;
    }
    let active = stats(&samples, true);
    let other = stats(&samples, false);
    let p95_passed = [&active, &other]
        .iter()
        .all(|s| s["p95_ms"].as_f64().is_some_and(|v| v <= 100.));
    let last = kavychka_platform::process::sample()?;
    let first = baseline.ok_or("Missing measurement baseline")?;
    let report = json!({"format":2,"passed":p95_passed,"release":true,
        "app_version":crate::app_version(),
        "unix_started_seconds":unix_started_seconds,
        "ui_scheduling_hint":scheduling.as_ref().map(|s|json!({"group":s.group,"preferred":s.preferred,"hard_affinity_changed":false,"cpu_sets_exclude_cpu0":s.selected_count!=0,"selected_cpu_count":s.selected_count})),
        "p95_passed":p95_passed,
        "tail_over_100_ms":active["over_100_ms"].as_u64().unwrap_or(0)+other["over_100_ms"].as_u64().unwrap_or(0),
        "exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e| e.to_string())?,
        "scope":"Actual AppWindow + settings service + installed text/speech worker. Background-thread action timestamp to software layout/render completion, 1100x780 full NewBuffer frames. No OS window, native input/compositor, or audio playback. Not an idle CPU benchmark.",
        "warmup_seconds":60,"request_interval_ms":50,"readiness_ms":ready,
        "active_ml_frames":active,"other_frames":other,
        "text_model":prefs.generation.text_model,"voice_model":prefs.generation.tts_model,
        "cpu_100ns_since_warmup":last.cpu_100ns.saturating_sub(first.cpu_100ns),
        "memory_scope":"UI/service process only; child-tree sampling supplied separately",
        "completed_job_recordings":completed_rows,"decoded_recordings":decoded,
        "cancel_ms":cancel_ms,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "physical_P4_tested":false,"target_laptop_tested":false,"24h_tested":false,
        "weights_downloaded":false,"live_sqlite_opened":false,"native_windows_created":false,
        "samples":samples,"process_samples":memory,
        "watchdog":probe.samples(),"watchdog_scope":"Owned UI thread CPU/IO and process counters, phases 1=callbacks/timers 2=render. Heartbeat gaps include watcher scheduling; no thread suspension.",
        "status_transition_count":statuses.len()});
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if !p95_passed {
        return Err("UI software-frame p95 exceeded 100 ms; see report".into());
    }
    Ok(())
}

/// Controlled contention regression using the real UI and production Job policy.
/// Only this test's UI thread and load process share one CPU; no native windows.
pub fn contention(output: PathBuf, fixture: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    use std::process::{Child, Command, Stdio};
    struct Load {
        child: Child,
        job: Option<kavychka_platform::process::Job>,
    }
    impl Drop for Load {
        fn drop(&mut self) {
            drop(self.job.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    if cfg!(debug_assertions)
        || !output.is_absolute()
        || output.exists()
        || !fixture.is_absolute()
        || !fixture.is_file()
    {
        return Err("Release, new absolute evidence and explicit load fixture required".into());
    }
    std::fs::create_dir_all(&output)?;
    let data = output.join("data");
    {
        let mut store = Sandbox::open(&data)?;
        let mut prefs = store.load()?;
        prefs.desktop.setup_complete = true;
        prefs.desktop.automatic_updates = false;
        prefs.desktop.run_at_login = false;
        prefs.generation.auto_daily = false;
        prefs.generation.text_enabled = false;
        prefs.generation.tts_model = "system".into();
        prefs.reminders.audio = crate::AudioMode::Silent;
        store.save(&prefs)?;
    }
    let window = MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::NewBuffer,
    );
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    let backend = Backend::start(data, ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 780);
    let pump = || {
        loop {
            let next = events.lock().unwrap().pop_front();
            match next {
                Some(next) => next(),
                None => break,
            }
        }
        slint::platform::update_timers_and_animations();
    };
    let ready = Instant::now();
    while !ui.get_ready() || ui.get_library_json().is_empty() {
        pump();
        if ready.elapsed() > Duration::from_secs(10) || ui.get_failed() {
            return Err("Contention UI startup failed".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Warm the same navigation pages before either case, avoiding first-layout bias.
    for page in [0, 2, 3, 5, 7] {
        ui.set_page(page);
        window.request_redraw();
        pump();
        window.draw_if_needed(|r| {
            r.render(pixels.make_mut_slice(), 1100);
        });
    }
    let affinity = kavychka_platform::process::TestAffinity::one_core()?;
    let mask = affinity.mask();
    let mut cases = vec![];
    for background in [false, true] {
        let name = if background {
            "below_normal_workers"
        } else {
            "normal_workers"
        };
        let mut command = Command::new(&fixture);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(output.join(format!("{name}.log")))?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut load = Load {
            child: command.spawn()?,
            job: None,
        };
        load.job = Some(if background {
            kavychka_platform::process::Job::assign_background(&load.child)?
        } else {
            kavychka_platform::process::Job::assign(&load.child)?
        });
        let priority = affinity.child_priority(&load.child)?;
        if priority != if background { 0x4000 } else { 0x20 } {
            return Err("Actual worker priority differs from policy".into());
        }
        let mut line = String::new();
        std::io::BufReader::new(load.child.stdout.take().ok_or("Load stdout missing")?)
            .read_line(&mut line)?;
        if line.trim() != "READY" {
            return Err("Load fixture failed its readiness handshake".into());
        }
        let (sent, received) = mpsc::sync_channel::<Instant>(1);
        let (ack, wait_ack) = mpsc::sync_channel::<()>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let weak = ui.as_weak();
        let producer = Producer {
            stop,
            join: Some(std::thread::spawn(move || {
                let mut index = 0;
                while !stopping.load(Ordering::Acquire) {
                    let queued = Instant::now();
                    let weak = weak.clone();
                    let sent = sent.clone();
                    let page = [0, 2, 3, 5, 7][index % 5];
                    index += 1;
                    if slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_page(page);
                            let _ = sent.try_send(queued);
                        }
                    })
                    .is_err()
                    {
                        break;
                    }
                    loop {
                        match wait_ack.recv_timeout(Duration::from_millis(50)) {
                            Ok(()) => break,
                            Err(mpsc::RecvTimeoutError::Timeout)
                                if !stopping.load(Ordering::Acquire) => {}
                            _ => return,
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50).saturating_sub(queued.elapsed()));
                }
            })),
        };
        load.child
            .stdin
            .take()
            .ok_or("Load stdin missing")?
            .write_all(b"run")?;
        let start = Instant::now();
        let mut measured = vec![];
        while start.elapsed() < Duration::from_secs(15) && measured.len() < 200 {
            pump();
            if let Ok(queued) = received.try_recv() {
                window.request_redraw();
                let mut rendered = false;
                window.draw_if_needed(|r| {
                    r.render(pixels.make_mut_slice(), 1100);
                    rendered = true;
                });
                if !rendered {
                    return Err("Contention frame was not rendered".into());
                }
                let elapsed = queued.elapsed().as_secs_f64() * 1000.;
                measured.push(
                    json!({"ml_running":false,"queue_to_frame_ms":elapsed,"page":ui.get_page()}),
                );
                ack.send(())?;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(load);
        drop(producer);
        let summary = stats(&measured, false);
        cases.push(json!({"case":name,"actual_priority_class":priority,"seconds":start.elapsed().as_secs_f64(),"statistics":summary,"samples":measured}));
    }
    affinity.restore()?;
    drop(backend);
    let normal = &cases[0]["statistics"];
    let below = &cases[1]["statistics"];
    let passed = normal["max_ms"].as_f64().is_some_and(|v| v > 100.)
        && below["count"].as_u64().is_some_and(|v| v >= 100)
        && below["over_100_ms"] == 0
        && below["p95_ms"].as_f64().is_some_and(|v| v < 50.);
    let report = json!({"passed":passed,"format":1,"app_version":crate::app_version(),"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e|e.to_string())?,"cpu_mask":mask,"thread_affinity_restored":true,"load_threads":48,"cases":cases,"scope":"Real AppWindow/software frames and production Job policy; owned UI thread and synthetic worker deliberately share ONE CPU. No models, native windows, system-wide settings or user data. This reproduces CPU starvation, not proof of the exact cause of a historic isolated sample."});
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if !passed {
        return Err(
            "Contention regression did not meet its explicit baseline/fixed criteria".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentile_is_nearest_rank_and_filters_other_mode() {
        let mut samples: Vec<_> = (1..=100)
            .map(|v| json!({"ml_running":true,"queue_to_frame_ms":v}))
            .collect();
        samples.push(json!({"ml_running":false,"queue_to_frame_ms":999}));
        let measured = stats(&samples, true);
        assert_eq!(measured["count"], 100);
        assert_eq!(measured["p95_ms"], 95.);
        assert_eq!(measured["max_ms"], 100.);
        assert_eq!(measured["over_100_ms"], 0);
    }
    #[test]
    fn no_samples_cannot_satisfy_a_latency_budget() {
        let measured = stats(&[], true);
        assert_eq!(measured["count"], 0);
        assert!(measured["p95_ms"].as_f64().is_none());
    }
}
