//! Opt-in cold setup through the real UI callback and production worker chain.
//! Creates only a fresh explicit QA directory. No OS window, sound or live data.
use crate::{AppWindow, Preferences, Sandbox, service::Backend};
use serde_json::{Value, json};
use slint::{
    ComponentHandle,
    platform::{EventLoopProxy, Platform, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
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
fn screenshot(
    window: &MinimalSoftwareWindow,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
fn fingerprints(root: &Path) -> Result<BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    fn walk(
        base: &Path,
        path: &Path,
        out: &mut BTreeMap<String, Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        kavychka_storage::preview::no_link_chain(path)?;
        for entry in std::fs::read_dir(path)? {
            let p = entry?.path();
            kavychka_storage::preview::no_link_chain(&p)?;
            if p.is_dir() {
                walk(base, &p, out)?;
            } else {
                let meta = p.metadata()?;
                out.insert(p.strip_prefix(base)?.to_string_lossy().replace('\\',"/"),json!({"bytes":meta.len(),"modified":meta.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_string(),"sha256":kavychka_maintenance::digest(&p).map_err(|e|e.to_string())?}));
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    if root.exists() {
        walk(root, root, &mut result)?;
    }
    Ok(result)
}
fn log_sizes(root: &Path) -> BTreeMap<String, u64> {
    ["runtime-install.log", "cosy-install.log"]
        .into_iter()
        .filter_map(|name| {
            root.join(name)
                .metadata()
                .ok()
                .map(|m| (name.into(), m.len()))
        })
        .collect()
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute QA output directory required".into());
    }
    std::fs::create_dir(&output)?;
    let result = execute(&output);
    if let Err(error) = &result {
        std::fs::write(
            output.join("failure.json"),
            serde_json::to_vec_pretty(
                &json!({"passed":false,"error":error.to_string(),"working_installation_changed":false}),
            )?,
        )?;
    }
    result
}
fn execute(output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let app = executable
        .parent()
        .ok_or("Missing packaged app directory")?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(app.join("assets/components.json"))?)?;
    let mut expected_model_files = BTreeSet::new();
    for id in ["text-gemma4-12b", "tts-cosyvoice3"] {
        for spec in manifest["models"][id]["files"]
            .as_array()
            .ok_or("Missing selected model manifest")?
        {
            expected_model_files.insert(
                Path::new(id)
                    .join(spec["path"].as_str().ok_or("Missing model filename")?)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        expected_model_files.insert(
            Path::new(id)
                .join("verified.json")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut prefs = Preferences::default();
    prefs.desktop.setup_complete = true;
    prefs.desktop.automatic_updates = false;
    prefs.desktop.run_at_login = false;
    prefs.generation.text_model = "text-gemma4-12b".into();
    prefs.generation.runtime_text_model = "text-gemma4-12b".into();
    prefs.generation.tts_model = "tts-cosyvoice3".into();
    prefs.generation.model_paths.clear();
    prefs.generation.voice_profile.clear();
    prefs.generation.device = "cpu".into();
    prefs.generation.text_enabled = true;
    prefs.generation.auto_daily = false;
    prefs.generation.runtime = false;
    prefs.generation.ac_only = false;
    prefs.generation.batch_count = 1;
    prefs.generation.categories = vec!["break".into()];
    prefs.schedule.work_days.clear();
    prefs.schedule.exercise_enabled = false;
    prefs.reminders.audio = crate::AudioMode::Silent;
    let mut initial = prefs.clone();
    initial.generation.text_enabled = false;
    initial.generation.tts_model = "system".into();
    store.save(&initial)?;
    drop(store);
    assert!(!data.join("components/runtime").exists());
    assert!(!data.join("components/models").exists());
    assert!(!data.join("hf").exists());
    let window = MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::NewBuffer,
    );
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    ui.set_page(3);
    let started = Instant::now();
    let mut phase = 0;
    let mut saw_running = false;
    let mut stage_started = Instant::now();
    let mut first_seconds = 0.;
    let mut second_seconds = 0.;
    let mut status_history = vec![];
    let mut last_status = String::new();
    let mut last_progress = Instant::now();
    let mut before_models = BTreeMap::new();
    let mut before_downloads = BTreeMap::new();
    let mut before_logs = BTreeMap::new();
    let recording_row;
    loop {
        pump(&events);
        if ui.get_failed() {
            return Err(ui.get_status().to_string().into());
        }
        if started.elapsed() > Duration::from_secs(2400) {
            return Err("Cold model setup qualification exceeded 40 minutes; not passed".into());
        }
        let status = ui.get_library_status().to_string();
        if status != last_status {
            status_history.push(
                json!({"seconds":started.elapsed().as_secs_f64(),"phase":phase,"status":status}),
            );
            last_status = status;
        }
        if last_progress.elapsed() > Duration::from_secs(3) {
            std::fs::write(
                output.join("progress.json"),
                serde_json::to_vec_pretty(
                    &json!({"phase":phase,"elapsed_seconds":started.elapsed().as_secs_f64(),"running":ui.get_generation_running(),"status":last_status,"passed":false}),
                )?,
            )?;
            last_progress = Instant::now();
        }
        if !ui.get_ready() || ui.get_library_json().is_empty() {
            if started.elapsed() > Duration::from_secs(30) {
                return Err("Cold UI readiness timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let view: crate::models_library::View =
            serde_json::from_str(ui.get_library_json().as_str())?;
        match phase {
            0 => {
                assert!(!crate::inference::selected_unavailable(&view.models, &prefs).is_empty());
                // Change model choices without the global Save button. Keep an
                // unrelated draft to prove setup does not commit or discard it.
                ui.set_text_model_index(0);
                ui.set_runtime_model_index(0);
                ui.set_voice_model_index(1);
                ui.set_text_enabled(true);
                ui.set_user_name("UNSAVED unrelated draft".into());
                ui.invoke_edited();
                assert!(ui.get_dirty());
                screenshot(&window, &output.join("before-install.png"))?;
                ui.invoke_install_models();
                phase = 1;
                stage_started = Instant::now();
                saw_running = false;
            }
            1 => {
                saw_running |= view.running;
                if saw_running && !view.running {
                    if !crate::inference::selected_unavailable(&view.models, &prefs).is_empty() {
                        return Err(format!(
                            "First cold installation did not finish: {}",
                            view.status
                        )
                        .into());
                    }
                    first_seconds = stage_started.elapsed().as_secs_f64();
                    let committed: Preferences =
                        serde_json::from_str(ui.get_loaded_prefs().as_str())?;
                    assert!(committed.generation.text_enabled);
                    assert_eq!(committed.generation.tts_model, "tts-cosyvoice3");
                    assert_eq!(ui.get_user_name(), "UNSAVED unrelated draft");
                    assert_eq!(committed.user_name, initial.user_name);
                    assert!(ui.get_dirty(), "Unrelated draft was discarded");
                    ui.set_user_name(initial.user_name.as_str().into());
                    ui.set_dirty(false);
                    screenshot(&window, &output.join("first-ready.png"))?;
                    before_models = fingerprints(&data.join("components/models"))?;
                    before_downloads = fingerprints(&data.join("components/downloads"))?;
                    before_logs = log_sizes(&data.join("components/logs"));
                    assert_eq!(
                        before_models.keys().cloned().collect::<BTreeSet<_>>(),
                        expected_model_files,
                        "Expected only selected pinned files and their verification markers"
                    );
                    ui.invoke_install_models();
                    phase = 2;
                    stage_started = Instant::now();
                    saw_running = false;
                } else if !saw_running && stage_started.elapsed() > Duration::from_secs(15) {
                    return Err(format!(
                        "Install button did not start its worker: {}",
                        view.status
                    )
                    .into());
                }
            }
            2 => {
                saw_running |= view.running;
                if saw_running && !view.running {
                    if !crate::inference::selected_unavailable(&view.models, &prefs).is_empty() {
                        return Err(
                            format!("Repeat installation did not finish: {}", view.status).into(),
                        );
                    }
                    second_seconds = stage_started.elapsed().as_secs_f64();
                    assert_eq!(
                        before_models,
                        fingerprints(&data.join("components/models"))?,
                        "Repeat button changed/downloaded model files"
                    );
                    assert_eq!(
                        before_downloads,
                        fingerprints(&data.join("components/downloads"))?,
                        "Repeat button downloaded components again"
                    );
                    assert_eq!(
                        before_logs,
                        log_sizes(&data.join("components/logs")),
                        "Repeat button reinstalled dependencies"
                    );
                    ui.set_page(2);
                    ui.invoke_generate_batch();
                    phase = 3;
                    stage_started = Instant::now();
                    saw_running = false;
                } else if !saw_running && stage_started.elapsed() > Duration::from_secs(15) {
                    return Err(
                        format!("Repeat button did not start its worker: {}", view.status).into(),
                    );
                }
            }
            3 => {
                saw_running |= view.running;
                if saw_running && !view.running {
                    let row = view
                        .phrases
                        .iter()
                        .find(|r| r.audio_path.is_some())
                        .ok_or_else(|| format!("No generated recording: {}", view.status))?;
                    recording_row = row.clone();
                    screenshot(&window, &output.join("generated-recording.png"))?;
                    break;
                } else if !saw_running && stage_started.elapsed() > Duration::from_secs(15) {
                    return Err(format!("Generate button did not start: {}", view.status).into());
                }
            }
            _ => unreachable!(),
        }
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1100, 780);
        window.draw_if_needed(|r| {
            r.render(pixels.make_mut_slice(), 1100);
        });
        std::thread::sleep(Duration::from_millis(15));
    }
    drop(backend);
    drop(ui);
    let store = Sandbox::open(&data)?;
    assert_eq!(store.root, data);
    // The service owns an exclusive storage lease until Backend::drop joins it.
    // Validate the recording only after that lease has been released.
    let file = store
        .audio_file(&recording_row)?
        .ok_or("Generated recording is unavailable")?;
    let pcm = kavychka_platform::audio::decode_recording(&file, &|| false)?;
    assert!(pcm.len() > 16_000, "Generated recording is too short");
    let decoded = json!({"decoded":true,"mono_pcm16_samples":pcm.len()/2,"text_nonempty":!recording_row.text.trim().is_empty(),"file_sha256":kavychka_maintenance::digest(&file).map_err(|e|e.to_string())?});
    let count = store.phrases()?.len();
    drop(store);
    let report = json!({"passed":true,"scope":"Real AppWindow callbacks + settings service + bootstrap/managed Python/worker on initially empty data. Public downloads, no seeded runtimes/models/HF cache. Software window only; no physical input or playback.","app_version":crate::app_version(),"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e|e.to_string())?,"models":["text-gemma4-12b","tts-cosyvoice3"],"device":"cpu","first_install_seconds":first_seconds,"repeat_seconds":second_seconds,"model_file_fingerprints":before_models,"repeat_model_and_download_fingerprints_equal":true,"repeat_dependency_log_sizes_equal":true,"generated_phrase_count":count,"recording":decoded,"status_history":status_history,"elapsed_seconds":started.elapsed().as_secs_f64(),"hardware_acceptance_performed":false,"audio_played":false,"native_windows_created":false,"live_profile_used":false});
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
