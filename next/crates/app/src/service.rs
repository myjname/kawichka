//! Sole SQLite/settings owner and clock-driven domain coordinator. UI callbacks
//! receive only committed views. ML is intentionally outside this base service.
use super::{AppWindow, models_library::Manager, publish};
use kavychka_core::{
    activity::Input,
    engine::{Clock, Effect, Engine, Environment, Notice},
    preferences::{Language, Preferences},
    reminders::Event,
};
use kavychka_platform::{
    audio, desktop,
    input::{Monitor, Observation},
    time,
};
use kavychka_storage::sandbox::Sandbox;
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::{
    path::PathBuf,
    sync::mpsc::{self, SyncSender},
    thread::JoinHandle,
    time::Duration,
};
pub enum Command {
    Profile(crate::data_ops::Action),
    Save(Box<Preferences>, Box<Preferences>),
    Reload,
    Confirm(String),
    Presented(String),
    Breakfast,
    Stretch,
    Pause(i32),
    Review,
    Silence,
    TestAudio {
        device: String,
        language: Language,
        mode: kavychka_core::preferences::AudioMode,
    },
    TogglePet,
    MovePet(i32, i32),
    PrepareDay,
    GenerateBatch,
    GenerateBatchOnBattery,
    RegenerateSpeech(Option<String>),
    SelectPhrase(String, bool),
    HidePhrase(String),
    LibrarySelection(crate::library_actions::Selection, String),
    PlayPhrase(String),
    PrepareModels(
        Box<kavychka_core::generation::Generation>,
        Box<kavychka_core::generation::Generation>,
    ),
    RepairEnvironment,
    RefreshLibrary,
    LibraryPage(bool, usize),
    ArchivePage(usize),
    ConfirmArchived(String),
    ExportLibrary,
    ClearRecordings,
    Preset(Box<crate::presets_ui::Action>),
    ControlledStep {
        clock: Clock,
        environment: Environment,
        events: Option<Vec<Event>>,
        input: Vec<(u64, Input)>,
    },
    Stop,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RuntimeView {
    #[serde(default)]
    pub presentation_requests: u64,
    #[serde(default)]
    pub audio_stop_requests: u64,
    #[serde(default)]
    pub on_battery: bool,
    pub prefs: Preferences,
    pub paused: bool,
    pub countdown: String,
    pub pending: usize,
    pub notice: Option<Notice>,
    pub next: String,
    pub status: String,
    pub breakfast_done: bool,
    pub pet_position: Option<(i32, i32)>,
    pub pet_permitted: bool,
    pub animation: bool,
    pub revision: u64,
    pub handled: usize,
    pub skipped: usize,
    pub audio_busy: bool,
    pub controlled_time: Option<i64>,
}
pub struct Backend {
    pub sender: SyncSender<Command>,
    pub cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    join: Option<JoinHandle<()>>,
}
enum Mode {
    Settings,
    Live,
    Controlled(Clock),
    Qualification(Clock),
}
impl Backend {
    pub fn start(path: PathBuf, ui: slint::Weak<AppWindow>) -> Self {
        Self::spawn(path, ui, Mode::Settings, None, None)
    }
    pub fn start_runtime(path: PathBuf, ui: slint::Weak<AppWindow>) -> Self {
        Self::spawn(path, ui, Mode::Live, None, None)
    }
    pub fn start_with_legacy(path: PathBuf, source: PathBuf, ui: slint::Weak<AppWindow>) -> Self {
        Self::spawn(path, ui, Mode::Live, Some(source), None)
    }
    pub fn start_controlled(path: PathBuf, ui: slint::Weak<AppWindow>, clock: Clock) -> Self {
        Self::spawn(path, ui, Mode::Controlled(clock), None, None)
    }
    pub fn start_qualification(path: PathBuf, ui: slint::Weak<AppWindow>, clock: Clock) -> Self {
        Self::spawn(path, ui, Mode::Qualification(clock), None, None)
    }
    /// Opt-in headless measurement: normal settings service, isolated data and
    /// explicitly selected read-only components. No legacy SQLite import.
    pub fn start_measurement(
        path: PathBuf,
        resources: PathBuf,
        ui: slint::Weak<AppWindow>,
    ) -> Self {
        Self::spawn(path, ui, Mode::Settings, None, Some(resources))
    }
    fn spawn(
        path: PathBuf,
        ui: slint::Weak<AppWindow>,
        mode: Mode,
        source: Option<PathBuf>,
        resources: Option<PathBuf>,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel(32);
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let user_cancellation = cancellation.clone();
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = shutdown.clone();
        let join = std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                let mut store = Sandbox::open(&path).map_err(|e| e.to_string())?;
                kavychka_storage::profile::cleanup_jobs(&store.root).map_err(|e| e.to_string())?;
                let mut library = Manager::new(&mut store, source.as_deref(), stop.clone())?;
                library.cancellation = user_cancellation.clone();
                if let Some(resources) = resources {
                    library.set_measurement_resources(resources);
                }
                let mut data_ops = crate::data_ops::Manager::new();
                let default_export = store.root.join("exports").join(format!(
                    "Kavychka-profile-{}.zip",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ));
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    ui.set_profile_export_path(crate::display_path::display(&default_export).into())
                });
                let prefs = store.load().map_err(|e| e.to_string())?;
                let runtime = !matches!(mode, Mode::Settings);
                let controlled = matches!(mode, Mode::Controlled(_) | Mode::Qualification(_));
                let mut clock = match mode {
                    Mode::Controlled(c) | Mode::Qualification(c) => c,
                    _ => time::now_in(&prefs.desktop.time_zone).map_err(|e| e.to_string())?,
                };
                let mut engine =
                    Engine::new(prefs, store.load_state().map_err(|e| e.to_string())?, clock)?;
                let mut warning = String::new();
                let mut audio_test_pending = false;
                let monitor = if matches!(mode, Mode::Live) {
                    match Monitor::start() {
                        Ok(m) => Some(m),
                        Err(e) => {
                            warning = format!("Activity adapter unavailable: {e}");
                            None
                        }
                    }
                } else {
                    None
                };
                let (audio_events_tx, audio_events_rx) = mpsc::channel::<audio::PlaybackEvent>();
                let output = if matches!(mode, Mode::Live | Mode::Qualification(_)) {
                    let fault_observer = crate::p4_crash::audio_observer();
                    Some(audio::Output::with_event_observer(Some(
                        std::sync::Arc::new(move |event| {
                            if let Some(observer) = &fault_observer {
                                observer(event.phase);
                            }
                            let _ = audio_events_tx.send(event);
                        }),
                    )))
                } else {
                    None
                };
                let mut env = if matches!(mode, Mode::Live) {
                    desktop::environment()
                } else {
                    Environment::default()
                };
                if runtime && (controlled || engine.prefs.desktop.setup_complete) {
                    engine.schedule(
                        time::day_in(
                            &engine.prefs.schedule,
                            clock,
                            &engine.prefs.desktop.time_zone,
                        )
                        .map_err(|e| e.to_string())?,
                        clock,
                        false,
                    );
                    crate::p4_crash::hit("recovery_before_checkpoint");
                    store
                        .save_runtime(&engine.prefs, &engine.state)
                        .map_err(|e| e.to_string())?;
                    crate::p4_crash::hit("recovery_after_checkpoint");
                }
                publish(&ui, Ok(engine.prefs.clone()), false);
                let mut stored = serde_json::to_string(&engine.state).map_err(|e| e.to_string())?;
                let mut last_view = String::new();
                let mut revision = 0;
                let mut presentation_requests = 0u64;
                let mut audio_stop_requests = 0u64;
                let mut date = clock.date;
                let mut offset = time::offset_in(clock.utc, &engine.prefs.desktop.time_zone)
                    .map_err(|e| e.to_string())?;
                loop {
                    if stop.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    let mut command = match receiver.recv_timeout(Duration::from_millis(250)) {
                        Ok(Command::Stop) => break,
                        Ok(c) => Some(c),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(_) => break,
                    };
                    if !controlled {
                        clock = time::now_in(&engine.prefs.desktop.time_zone)
                            .map_err(|e| e.to_string())?;
                    }
                    if matches!(mode, Mode::Live) {
                        env = desktop::environment();
                    }
                    // Honour a queued cancel before polling a prepared restore:
                    // it must not cross the SQLite commit just ahead of Cancel.
                    if matches!(
                        &command,
                        Some(Command::Profile(
                            crate::data_ops::Action::Cancel | crate::data_ops::Action::Discard
                        ))
                    ) && let Some(Command::Profile(action)) = command.take()
                        && let Err(error) = data_ops.action(action, &store, library.running())
                    {
                        data_ops.fail(error);
                    }
                    data_ops.poll();
                    if let Some(plan) = data_ops.take_ready() {
                        if let Some(output) = &output {
                            output.stop();
                        }
                        let result = plan
                            .apply(&mut store, &std::sync::atomic::AtomicBool::new(false))
                            .map_err(|e| e.to_string());
                        if result.is_ok() {
                            let prefs = store.load().map_err(|e| e.to_string())?;
                            if !controlled {
                                clock = time::now_in(&prefs.desktop.time_zone)
                                    .map_err(|e| e.to_string())?;
                            }
                            engine = Engine::new(
                                prefs,
                                store.load_state().map_err(|e| e.to_string())?,
                                clock,
                            )?;
                            engine.schedule(
                                time::day_in(
                                    &engine.prefs.schedule,
                                    clock,
                                    &engine.prefs.desktop.time_zone,
                                )
                                .map_err(|e| e.to_string())?,
                                clock,
                                true,
                            );
                            store
                                .save_runtime(&engine.prefs, &engine.state)
                                .map_err(|e| e.to_string())?;
                            stored =
                                serde_json::to_string(&engine.state).map_err(|e| e.to_string())?;
                            library = Manager::new(&mut store, None, stop.clone())?;
                            library.cancellation = user_cancellation.clone();
                            last_view.clear();
                            publish(&ui, Ok(engine.prefs.clone()), false);
                            let _ = ui.upgrade_in_event_loop(|ui| {
                                ui.set_profile_operation_index(1);
                                ui.set_page(6);
                            });
                        }
                        data_ops.applied(result);
                    }
                    data_ops.publish(&ui);
                    library.set_data_hold(data_ops.blocked());
                    let mut candidate = engine.clone();
                    let confirmation_key = match &command {
                        Some(Command::Confirm(key)) => Some(key.clone()),
                        _ => None,
                    };
                    let confirming = confirmation_key.is_some();
                    let runtime = runtime && (controlled || candidate.prefs.desktop.setup_complete);
                    let mut effect = Effect::default();
                    let mut preferences_reply = None;
                    let mut prepare_models_reply = None;
                    let mut character_import = false;
                    for event in audio_events_rx.try_iter() {
                        if let Err(error) = journal_audio(&mut store, event) {
                            if let Some(output) = &output {
                                output.stop();
                            }
                            warning =
                                format!("Delivery journal unavailable; audio stopped: {error}");
                        }
                    }
                    if let Some(monitor) = &monitor {
                        monitor.update_environment(&mut env);
                        for observation in monitor.drain() {
                            match observation {
                                Observation::Input(at, event) => candidate.input(at, event),
                                Observation::Suspension(at, duration) => {
                                    candidate.suspension(at, duration)
                                }
                                Observation::Uncertain(at)
                                | Observation::SessionChanged(at)
                                | Observation::Suspending(at) => candidate.uncertain_input(at),
                                Observation::PowerChanged(_) => {
                                    env.power = kavychka_platform::power_source()
                                }
                            }
                        }
                    } else if matches!(mode, Mode::Live) {
                        candidate.uncertain_input(clock.mono);
                    }
                    if let Some(output) = &output
                        && let Some(feedback) = output.feedback()
                    {
                        if let Err(error) = &feedback {
                            warning = error.clone();
                        }
                        if audio_test_pending {
                            let error = feedback.err();
                            let _ = ui.upgrade_in_event_loop(move |ui| {
                                ui.set_audio_test_status(
                                    error
                                        .unwrap_or_else(|| {
                                            if ui.get_english() {
                                                "Sound test completed".into()
                                            } else {
                                                "Проверка звука завершена".into()
                                            }
                                        })
                                        .into(),
                                )
                            });
                            audio_test_pending = false;
                        }
                    }
                    if let Some(command) = command {
                        let auto_import_requested = matches!(&command,Command::Preset(action) if matches!(action.as_ref(),crate::presets_ui::Action::ImportAndApply(_)));
                        let library_command = matches!(
                            &command,
                            Command::PrepareDay
                                | Command::GenerateBatch
                                | Command::GenerateBatchOnBattery
                                | Command::RegenerateSpeech(_)
                                | Command::SelectPhrase(..)
                                | Command::HidePhrase(_)
                                | Command::LibrarySelection(..)
                                | Command::PlayPhrase(_)
                                | Command::PrepareModels(..)
                                | Command::RepairEnvironment
                                | Command::ExportLibrary
                                | Command::ClearRecordings
                                | Command::Preset(_)
                        );
                        let result = (|| -> Result<(), String> {
                            if data_ops.blocked()
                                && library_command
                                && !matches!(
                                    &command,
                                    Command::PlayPhrase(_) | Command::RefreshLibrary
                                )
                            {
                                return Err("Заверши или отмени операцию с данными / Finish or cancel the data operation".into());
                            }
                            match command {
                                Command::Profile(action) => {
                                    if let Err(error) = data_ops.action(
                                        action,
                                        &store,
                                        library.running() || library.waiting(),
                                    ) {
                                        data_ops.fail(error);
                                    }
                                    data_ops.publish(&ui);
                                    library.set_data_hold(data_ops.blocked());
                                }
                                Command::Save(desired, base) => {
                                    if data_ops.blocked() {
                                        return Err("Заверши или отмени операцию с данными / Finish or cancel the data operation".into());
                                    }
                                    let prefs = kavychka_core::preferences::merge(
                                        &candidate.prefs,
                                        &base,
                                        &desired,
                                    )?;
                                    let generation_changed =
                                        serde_json::to_value(&prefs.generation)
                                            .map_err(|e| e.to_string())?
                                            != serde_json::to_value(&candidate.prefs.generation)
                                                .map_err(|e| e.to_string())?
                                            || prefs.speech_language
                                                != candidate.prefs.speech_language
                                            || prefs.user_name != candidate.prefs.user_name
                                            || serde_json::to_value(&prefs.schedule)
                                                .map_err(|e| e.to_string())?
                                                != serde_json::to_value(&candidate.prefs.schedule)
                                                    .map_err(|e| e.to_string())?
                                            || prefs.desktop.time_zone
                                                != candidate.prefs.desktop.time_zone;
                                    if candidate.prefs.schedule.exercise_enabled
                                        && !prefs.schedule.exercise_enabled
                                        || prefs.reminders.audio != candidate.prefs.reminders.audio
                                        || prefs.reminders.audio_device
                                            != candidate.prefs.reminders.audio_device
                                        || prefs.reminders.fullscreen_audio
                                            != candidate.prefs.reminders.fullscreen_audio
                                    {
                                        effect.stop_audio = true;
                                    }
                                    if !controlled {
                                        clock = time::now_in(&prefs.desktop.time_zone)
                                            .map_err(|e| e.to_string())?;
                                    }
                                    candidate.preferences(prefs, clock)?;
                                    if generation_changed {
                                        library.settings_changed(clock);
                                    }
                                    if !matches!(mode, Mode::Settings)
                                        && (controlled || candidate.prefs.desktop.setup_complete)
                                    {
                                        candidate.schedule(
                                            time::day_in(
                                                &candidate.prefs.schedule,
                                                clock,
                                                &candidate.prefs.desktop.time_zone,
                                            )
                                            .map_err(|e| e.to_string())?,
                                            clock,
                                            true,
                                        );
                                    }
                                    preferences_reply = Some(true);
                                    library.invalidate();
                                }
                                Command::Reload => {
                                    preferences_reply = Some(false);
                                }
                                Command::TestAudio {
                                    device,
                                    language,
                                    mode,
                                } => {
                                    let result = output.as_ref().ok_or("Audio tests are unavailable in controlled mode".to_owned())
                                        .and_then(|out| out.play_on(if language == Language::Ru { "Это пример напоминания. Можно сделать небольшую паузу." } else { "This is a reminder preview. Take a little break." }, language, mode, &device));
                                    match result {
                                        Ok(()) => {
                                            audio_test_pending = true;
                                        }
                                        Err(error) => {
                                            let _ = ui.upgrade_in_event_loop(move |ui| {
                                                ui.set_audio_test_status(error.into())
                                            });
                                        }
                                    }
                                }
                                Command::Preset(action) => {
                                    let apply_import = matches!(
                                        action.as_ref(),
                                        crate::presets_ui::Action::ImportAndApply(_)
                                    );
                                    let edited_description = match action.as_ref() {
                                        crate::presets_ui::Action::Save(id, preset)
                                            if *id == candidate.prefs.desktop.character_preset =>
                                        {
                                            preset.content.description().map(str::to_owned)
                                        }
                                        _ => None,
                                    };
                                    library.preset_action(&store, &candidate.prefs, *action)?;
                                    if apply_import {
                                        library.apply_imported(&store, &mut candidate.prefs)?;
                                        character_import = true;
                                    }
                                    candidate.prefs.desktop.visual_revision =
                                        candidate.prefs.desktop.visual_revision.wrapping_add(1);
                                    if let Some(description) = edited_description {
                                        candidate.prefs.generation.character_description =
                                            description;
                                    }
                                }
                                Command::Confirm(key) => {
                                    candidate.confirm(&key, clock)?;
                                    effect.stop_audio = true;
                                }
                                Command::Presented(key) => {
                                    if let Err(error) = store.delivery_phase(&key, "shown") {
                                        if let Some(output) = &output {
                                            output.stop();
                                        }
                                        return Err(format!(
                                            "Delivery journal unavailable; audio stopped: {error}"
                                        ));
                                    }
                                }
                                Command::Breakfast => candidate.breakfast(clock)?,
                                Command::Stretch => {
                                    if !env.session_blocked() {
                                        effect = candidate.manual_stretch(clock);
                                    }
                                }
                                Command::Pause(minutes) => {
                                    let until = match minutes {
                                        0 => None,
                                        -1 => Some(
                                            time::tomorrow_in(
                                                clock,
                                                &candidate.prefs.desktop.time_zone,
                                            )
                                            .map_err(|e| e.to_string())?,
                                        ),
                                        n if (1..=1440).contains(&n) => {
                                            Some(time::pause_minutes(clock, n as u32))
                                        }
                                        _ => return Err("invalid pause duration".into()),
                                    };
                                    candidate.pause(until);
                                    effect.stop_audio = true;
                                }
                                Command::Review => {
                                    if candidate.state.pending.is_empty()
                                        && library.archived_count > 0
                                    {
                                        let _ = ui.upgrade_in_event_loop(|ui| {
                                            ui.set_page(0);
                                            let _ = ui.show();
                                        });
                                    } else {
                                        candidate.review();
                                    }
                                }
                                Command::Silence => effect.stop_audio = true,
                                Command::TogglePet => {
                                    candidate.prefs.reminders.pet_visible =
                                        !candidate.prefs.reminders.pet_visible
                                }
                                Command::MovePet(x, y) => {
                                    candidate.state.pet_position = Some((x, y))
                                }
                                Command::PrepareDay => {
                                    check_generation(&candidate, clock, env, false)?;
                                    library.resume_preparation();
                                    library.generate(&mut store, &candidate.prefs, clock, true)?;
                                }
                                Command::GenerateBatch => {
                                    check_generation(&candidate, clock, env, false)?;
                                    library.generate(&mut store, &candidate.prefs, clock, false)?;
                                }
                                Command::GenerateBatchOnBattery => {
                                    check_generation(&candidate, clock, env, true)?;
                                    library.generate(&mut store, &candidate.prefs, clock, false)?;
                                    library.allow_current_task_on_battery();
                                }
                                Command::RegenerateSpeech(id) => {
                                    check_generation(&candidate, clock, env, false)?;
                                    library.speech(
                                        &mut store,
                                        &candidate.prefs,
                                        clock,
                                        id.as_deref(),
                                    )?;
                                }
                                Command::SelectPhrase(id, keep) => {
                                    store.select_phrase(&id, keep).map_err(|e| e.to_string())?;
                                    library.invalidate();
                                }
                                Command::LibrarySelection(selection, action) => {
                                    selection.validate(&clock.date.to_string(), &action)?;
                                    if action != "all" && (library.running() || library.waiting()) {
                                        return Err(
                                            "Дождись текущей задачи / Wait for the current task"
                                                .into(),
                                        );
                                    }
                                    let available = if selection.group == "support" {
                                        crate::support::load(
                                            &store,
                                            &candidate.prefs,
                                            &selection.day,
                                        )?
                                        .texts
                                    } else {
                                        store
                                            .phrases()
                                            .map_err(|e| e.to_string())?
                                            .into_iter()
                                            .filter(|r| {
                                                !r.deleted
                                                    && ((r.day.as_deref()
                                                        == Some(selection.day.as_str()))
                                                        == (selection.group == "current"))
                                            })
                                            .map(|r| r.id)
                                            .collect()
                                    };
                                    if action == "all" {
                                        let _ = ui.upgrade_in_event_loop(move |ui| {
                                            crate::library_actions::accept_all(
                                                &ui, &selection, available,
                                            )
                                        });
                                    } else {
                                        if selection.ids.iter().any(|id| !available.contains(id)) {
                                            return Err("Список изменился. Выбери фразы заново / List changed; select again".into());
                                        }
                                        let ids = selection.ids.iter().cloned().collect::<Vec<_>>();
                                        match action.as_str() {
                                            "delete" if selection.group == "support" => {
                                                library.remove_support(&store, &ids)?
                                            }
                                            "delete" => {
                                                store
                                                    .change_phrases(&ids, None)
                                                    .map_err(|e| e.to_string())?;
                                                library.status = format!(
                                                    "Удалено из списка: {}. Тексты сохранены для защиты от повторов / Removed; anti-repeat text retained",
                                                    ids.len()
                                                );
                                            }
                                            "keep" | "exclude" => {
                                                store
                                                    .change_phrases(&ids, Some(action == "keep"))
                                                    .map_err(|e| e.to_string())?;
                                                library.status = format!(
                                                    "Обработано фраз / Phrases updated: {}",
                                                    ids.len()
                                                );
                                            }
                                            "speech" => {
                                                check_generation(&candidate, clock, env, false)?;
                                                library.speech_selected(
                                                    &mut store,
                                                    &candidate.prefs,
                                                    clock,
                                                    Some(&ids),
                                                )?;
                                            }
                                            "export" => library.export_selected(
                                                &store,
                                                &candidate.prefs,
                                                clock,
                                                Some(&ids),
                                            )?,
                                            _ => return Err("Unknown library action".into()),
                                        }
                                        library.invalidate();
                                        let _ = ui.upgrade_in_event_loop(move |ui| {
                                            crate::library_actions::clear_if_unchanged(
                                                &ui, &selection,
                                            )
                                        });
                                    }
                                }
                                Command::PlayPhrase(id) => {
                                    let row = store.phrase(&id).map_err(|e| e.to_string())?;
                                    library.preview =
                                        match store.audio_file(&row).map_err(|e| e.to_string())? {
                                            Some(path) => {
                                                Some(super::models_library::Preview::File(path))
                                            }
                                            None if row.model == "system" => {
                                                Some(super::models_library::Preview::Text(
                                                    row.text,
                                                    if row.language == "en" {
                                                        Language::En
                                                    } else {
                                                        Language::Ru
                                                    },
                                                ))
                                            }
                                            None => return Err(
                                                "Аудиозапись ещё не готова / Recording not ready"
                                                    .into(),
                                            ),
                                        };
                                    if let Some(output) = &output {
                                        output.stop();
                                    }
                                }
                                Command::PrepareModels(desired, base) => {
                                    library.can_install()?;
                                    let prefs =
                                        merge_model_setup(&candidate.prefs, &base, &desired)?;
                                    let changed = serde_json::to_value(&prefs.generation)
                                        .map_err(|e| e.to_string())?
                                        != serde_json::to_value(&candidate.prefs.generation)
                                            .map_err(|e| e.to_string())?;
                                    candidate.preferences(prefs, clock)?;
                                    prepare_models_reply = Some((desired, base, changed));
                                    library.invalidate();
                                }
                                Command::RepairEnvironment => {
                                    library.repair_environment(&store, &candidate.prefs, clock)?
                                }
                                Command::RefreshLibrary => library.invalidate(),
                                Command::HidePhrase(id) => {
                                    if library.running() || library.waiting() {
                                        return Err(
                                            "Дождись текущей задачи / Wait for the current task"
                                                .into(),
                                        );
                                    }
                                    store.hide_phrase(&id).map_err(|e| e.to_string())?;
                                    library.status="Фраза удалена из списка. Текст сохранён для защиты от повторов / Phrase removed; text retained for anti-repeat".into();
                                    library.invalidate();
                                }
                                Command::LibraryPage(today, page) => library.browse(today, page),
                                Command::ArchivePage(page) => library.browse_archive(page),
                                Command::ConfirmArchived(key) => {
                                    store.confirm_archived(&key).map_err(|e| e.to_string())?;
                                    library.invalidate();
                                }
                                Command::ExportLibrary => {
                                    library.export(&store, &candidate.prefs, clock)?
                                }
                                Command::ClearRecordings => {
                                    effect.stop_audio = true;
                                    if library.running() {
                                        return Err(
                                            "Сначала отмени генерацию / Cancel generation first"
                                                .into(),
                                        );
                                    }
                                    let count = store
                                        .expire_audio(&clock.date.to_string(), true)
                                        .map_err(|e| e.to_string())?;
                                    library.status = format!(
                                        "Убрано аудиозаписей: {count}. Тексты сохранены / Texts retained"
                                    );
                                    library.invalidate();
                                }
                                Command::ControlledStep {
                                    clock: at,
                                    environment,
                                    events,
                                    input,
                                } if controlled => {
                                    clock = at;
                                    env = environment;
                                    if let Some(events) = events {
                                        candidate.schedule(events, clock, false);
                                    }
                                    for (at, event) in input {
                                        candidate.input(at, event);
                                    }
                                }
                                Command::ControlledStep { .. } => {
                                    return Err("controlled clock is disabled in live mode".into());
                                }
                                Command::Stop => unreachable!(),
                            }
                            Ok(())
                        })();
                        if let Err(error) = result {
                            if auto_import_requested {
                                library.import_aborted();
                            }
                            if library_command {
                                library.status = error;
                                library.invalidate();
                            } else {
                                publish(&ui, Err(error), false);
                                continue;
                            }
                        }
                    }
                    if runtime {
                        let new_offset =
                            time::offset_in(clock.utc, &engine.prefs.desktop.time_zone)
                                .map_err(|e| e.to_string())?;
                        if !controlled && (date != clock.date || offset != new_offset) {
                            candidate.schedule(
                                time::day_in(
                                    &candidate.prefs.schedule,
                                    clock,
                                    &candidate.prefs.desktop.time_zone,
                                )
                                .map_err(|e| e.to_string())?,
                                clock,
                                date == clock.date,
                            );
                            date = clock.date;
                            offset = new_offset;
                        }
                        let tick = candidate.tick(
                            clock,
                            env,
                            library.waiting() || output.as_ref().is_some_and(|a| a.busy()),
                        );
                        if effect.announce.is_none() {
                            effect.announce = tick.announce;
                        }
                        effect.stop_audio |= tick.stop_audio;
                    }
                    let allow_generation =
                        matches!(mode, Mode::Live) && candidate.prefs.desktop.setup_complete;
                    if let Err(error) = library.tick(
                        &mut store,
                        &mut candidate,
                        clock,
                        env,
                        &mut effect,
                        allow_generation,
                    ) {
                        library.fail(error, clock);
                    }
                    let fallback = effect.announce.clone();
                    let claim = match library.deliver(&store, &mut candidate, clock, &mut effect) {
                        Ok(claim) => claim,
                        Err(error) => {
                            library.status = error;
                            library.invalidate();
                            effect.announce = fallback;
                            None
                        }
                    };
                    if runtime
                        && candidate.state.checkpoint_utc.is_none_or(|saved| {
                            clock.utc - saved >= chrono::Duration::seconds(30) || saved > clock.utc
                        })
                    {
                        candidate.state.checkpoint_utc = Some(clock.utc);
                    }
                    let serialized =
                        serde_json::to_string(&candidate.state).map_err(|e| e.to_string())?;
                    let prefs_changed = serde_json::to_value(&candidate.prefs)
                        .map_err(|e| e.to_string())?
                        != serde_json::to_value(&engine.prefs).map_err(|e| e.to_string())?;
                    if serialized != stored
                        || !candidate.archived_pending.is_empty()
                        || prefs_changed
                        || preferences_reply.is_some_and(|saved| saved)
                    {
                        if claim.is_some() {
                            crate::p4_crash::hit("before_claim_commit");
                        }
                        if confirming {
                            crate::p4_crash::hit("before_confirm_commit");
                        }
                        let result = if runtime {
                            store.save_delivery_archived(
                                &candidate.prefs,
                                &candidate.state,
                                claim
                                    .as_ref()
                                    .map(|(notice, id, _)| (notice.as_str(), id.as_str())),
                                effect.announce.as_ref(),
                                confirmation_key.as_deref(),
                                &candidate.archived_pending,
                            )
                        } else {
                            store.save(&candidate.prefs)
                        };
                        if let Err(error) = result {
                            if character_import {
                                library.import_aborted();
                                library.status = format!(
                                    "Не удалось применить персонажа / Could not apply character: {error}"
                                );
                            }
                            if let Some(output) = &output {
                                output.stop();
                            }
                            publish(&ui, Err(error.to_string()), false);
                            continue;
                        }
                        stored = serialized;
                        if !candidate.archived_pending.is_empty() {
                            library.invalidate();
                            candidate.archived_pending.clear();
                        }
                    }
                    if preferences_reply == Some(true)
                        && std::env::var_os("KAVYCHKA_PORTABLE_ROOT").is_none()
                        && let Some(root) = std::env::var_os("KAVYCHKA_INSTALL_ROOT")
                        && let Err(error) = kavychka_maintenance::windows::startup(
                            &PathBuf::from(root),
                            candidate.prefs.desktop.run_at_login,
                        )
                    {
                        warning =
                            format!("Автозапуск не настроен / Startup integration failed: {error}");
                    }
                    engine = candidate;
                    if character_import {
                        library.import_committed();
                        library.settings_changed(clock);
                    }
                    if let Some((desired, base, changed)) = prepare_models_reply {
                        // The settings transaction above must succeed before any
                        // cancellation, environment installation or download.
                        if changed {
                            library.settings_changed(clock);
                        }
                        if let Err(error) = library.install(&store, &engine.prefs, clock) {
                            library.status = error;
                            library.invalidate();
                        }
                        let _ = ui.upgrade_in_event_loop(move |ui| {
                            crate::models_ui::setup_saved(&ui, &desired, &base);
                        });
                    }
                    if effect.announce.is_some() {
                        presentation_requests = presentation_requests.saturating_add(1);
                    }
                    if effect.stop_audio {
                        audio_stop_requests = audio_stop_requests.saturating_add(1);
                    }
                    if claim.is_some() {
                        crate::p4_crash::hit("after_claim_commit");
                    }
                    if confirming {
                        crate::p4_crash::hit("after_confirm_commit");
                    }
                    revision += 1;
                    // Physical effects only follow a successful state transaction.
                    if let Some(output) = &output {
                        if effect.stop_audio {
                            output.stop();
                            if audio_test_pending {
                                audio_test_pending = false;
                                let _ = ui.upgrade_in_event_loop(|ui| {
                                    ui.set_audio_test_status(
                                        if ui.get_english() {
                                            "Sound test stopped"
                                        } else {
                                            "Проверка звука остановлена"
                                        }
                                        .into(),
                                    )
                                });
                            }
                            library.preview = None;
                        }
                        if let Some(notice) = effect.announce
                            && !env.session_blocked()
                            && (!env.notifications_blocked(engine.prefs.reminders.allow_during_dnd)
                                || notice.explicit)
                        {
                            warning.clear();
                            let mode = engine.audio_mode(env);
                            let permission =
                                if mode == kavychka_core::preferences::AudioMode::Silent {
                                    Ok(())
                                } else {
                                    store
                                        .delivery_phase(&notice.key, "audio_requested")
                                        .map_err(|e| e.to_string())
                                };
                            let playback = permission.and_then(|()| {
                                match claim.as_ref().and_then(|(_, _, path)| path.clone()) {
                                    Some(path) => output.play_file_notice_on(
                                        path,
                                        mode,
                                        &engine.prefs.reminders.audio_device,
                                        &notice.key,
                                    ),
                                    None => output.play_notice_on(
                                        &notice.text,
                                        engine.prefs.speech_language,
                                        mode,
                                        &engine.prefs.reminders.audio_device,
                                        &notice.key,
                                    ),
                                }
                            });
                            if let Err(error) = playback {
                                warning = error;
                            }
                        }
                        if env.session_blocked() {
                            library.preview = None;
                        }
                        if !output.busy()
                            && let Some(preview) = library.preview.take()
                            && let Err(error) = match preview {
                                super::models_library::Preview::File(path) => output.play_file_on(
                                    path,
                                    kavychka_core::preferences::AudioMode::Voice,
                                    &engine.prefs.reminders.audio_device,
                                ),
                                super::models_library::Preview::Text(text, language) => output
                                    .play_on(
                                        &text,
                                        language,
                                        kavychka_core::preferences::AudioMode::Voice,
                                        &engine.prefs.reminders.audio_device,
                                    ),
                            }
                        {
                            library.status = error;
                            library.invalidate();
                        }
                    }
                    if let Some(json) = library.view(&store, &engine.prefs)? {
                        let _ = ui
                            .upgrade_in_event_loop(move |ui| ui.invoke_library_update(json.into()));
                    }
                    if let Some(saved) = preferences_reply {
                        publish(&ui, Ok(engine.prefs.clone()), saved);
                    }
                    if runtime {
                        let mut view = view(
                            &engine,
                            clock,
                            env,
                            &warning,
                            output.as_ref().is_some_and(|o| o.busy()),
                        );
                        view.pending += library.archived_count;
                        if controlled {
                            view.controlled_time = Some(clock.utc.timestamp_millis());
                        }
                        view.presentation_requests = presentation_requests;
                        view.audio_stop_requests = audio_stop_requests;
                        // Ignore revision for change detection: an idle service posts no UI work.
                        let json = serde_json::to_string(&view).map_err(|e| e.to_string())?;
                        if json != last_view {
                            last_view = json;
                            view.revision = revision;
                            let json = serde_json::to_string(&view).map_err(|e| e.to_string())?;
                            let _ = ui.upgrade_in_event_loop(move |ui| {
                                ui.invoke_runtime_update(json.into())
                            });
                        }
                    }
                }
                library.prepare_shutdown(&store, clock)?;
                if let Some(output) = &output {
                    output.stop();
                }
                drop(output);
                for event in audio_events_rx.try_iter() {
                    library.invalidate();
                    journal_audio(&mut store, event)?;
                }
                drop(monitor);
                Ok(())
            };
            if let Err(error) = run() {
                publish(&ui, Err(error), false);
                let _ = ui.upgrade_in_event_loop(|ui| {
                    ui.set_background_running(false);
                    ui.invoke_runtime_stopped();
                });
            }
        });
        Self {
            sender,
            cancellation,
            shutdown,
            join: Some(join),
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = self.sender.send(Command::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn merge_model_setup(
    current: &Preferences,
    base: &kavychka_core::generation::Generation,
    desired: &kavychka_core::generation::Generation,
) -> Result<Preferences, String> {
    let current_value = serde_json::to_value(&current.generation).map_err(|e| e.to_string())?;
    let base_value = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let desired_value = serde_json::to_value(desired).map_err(|e| e.to_string())?;
    let mut filtered = base_value.clone();
    for key in [
        "device",
        "text_model",
        "runtime_text_model",
        "tts_model",
        "voice_profile",
        "text_enabled",
        "runtime",
        "auto_daily",
        "daily_auto_keep",
        "ac_only",
        "model_paths",
        "runtime_wait_seconds",
    ] {
        if base_value[key] != desired_value[key]
            && current_value[key] != base_value[key]
            && current_value[key] != desired_value[key]
        {
            return Err("Выбор моделей уже изменился. Обнови страницу и выбери снова / Model choices changed; reload and choose again".into());
        }
        filtered[key] = desired_value[key].clone();
    }
    let mut baseline = current.clone();
    baseline.generation = base.clone();
    let mut wanted = baseline.clone();
    wanted.generation = serde_json::from_value(filtered).map_err(|e| e.to_string())?;
    kavychka_core::preferences::merge(current, &baseline, &wanted)
}
fn journal_audio(store: &mut Sandbox, event: audio::PlaybackEvent) -> Result<(), String> {
    if let Some(notice) = event.notice {
        let phase = match event.phase {
            audio::Phase::Preparing => "audio_preparing",
            audio::Phase::Started => "audio_started",
            audio::Phase::Completed => "audio_completed",
            audio::Phase::Cancelled => "audio_cancelled",
            audio::Phase::Failed => "audio_failed",
        };
        store
            .delivery_phase(&notice, phase)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn check_generation(
    engine: &Engine,
    clock: Clock,
    env: Environment,
    allow_battery_once: bool,
) -> Result<(), String> {
    if engine.paused(clock) {
        return Err("Генерация отложена паузой / Generation paused".into());
    }
    if env.fullscreen && engine.prefs.desktop.defer_generation_fullscreen {
        return Err("Генерация отложена полным экраном / Generation deferred by fullscreen".into());
    }
    if engine.prefs.generation.ac_only
        && env.power != kavychka_core::engine::PowerSource::Ac
        && !allow_battery_once
    {
        return Err("Подключи питание или явно разреши одну партию от батареи / Connect AC power or allow one battery batch".into());
    }
    Ok(())
}
fn view(
    engine: &Engine,
    clock: Clock,
    env: Environment,
    warning: &str,
    audio_busy: bool,
) -> RuntimeView {
    use kavychka_core::preferences::VisualMode;
    let en = engine.prefs.ui_language == Language::En;
    let paused = engine.paused(clock);
    let countdown = engine
        .state
        .pause_until
        .filter(|_| paused)
        .map(|until| {
            let seconds = (until - clock.utc).num_seconds().max(0);
            format!(
                "{} {:02}:{:02}",
                if en { "Pause" } else { "Пауза" },
                seconds / 60,
                seconds % 60
            )
        })
        .unwrap_or_default();
    let next = engine
        .next_event(clock)
        .map(|event| {
            format!(
                "{} · {}",
                time::caption_in(event.due, &engine.prefs.desktop.time_zone)
                    .unwrap_or_else(|_| "?".into()),
                category(event.category, en)
            )
        })
        .unwrap_or_else(|| {
            if en {
                "No scheduled events left today"
            } else {
                "На сегодня событий больше нет"
            }
            .into()
        });
    let status = if !warning.is_empty() {
        warning.into()
    } else if env.suspended {
        if en {
            "System suspended"
        } else {
            "Система переходит в сон"
        }
        .into()
    } else if env.session_unknown {
        if en {
            "Session unavailable · rest is not inferred"
        } else {
            "Сеанс недоступен · отдых не засчитывается"
        }
        .into()
    } else if env.locked {
        if en {
            "Screen locked"
        } else {
            "Экран заблокирован"
        }
        .into()
    } else if paused {
        countdown.clone()
    } else if env.notifications == kavychka_core::engine::NotificationState::Unknown {
        if en {
            "Windows notification state unavailable · use Pause"
        } else {
            "Состояние уведомлений Windows неизвестно · используй паузу"
        }
        .into()
    } else if env.notifications_blocked(engine.prefs.reminders.allow_during_dnd) {
        if en {
            "Notifications suppressed by the system"
        } else {
            "Уведомления ограничены системой"
        }
        .into()
    } else {
        if en {
            "Reminders enabled · DND integration limited"
        } else {
            "Напоминания включены · DND определяется с ограничениями"
        }
        .into()
    };
    RuntimeView {
        presentation_requests: 0,
        audio_stop_requests: 0,
        on_battery: env.power != kavychka_core::engine::PowerSource::Ac,
        prefs: engine.prefs.clone(),
        paused,
        countdown,
        pending: engine.state.pending.len(),
        notice: engine.current(clock, env).cloned(),
        next,
        status,
        breakfast_done: engine.state.breakfast_dates.contains(&clock.date),
        pet_position: engine.state.pet_position,
        pet_permitted: !env.session_blocked()
            && !env.notifications_blocked(engine.prefs.reminders.allow_during_dnd)
            && (!env.fullscreen || engine.prefs.reminders.fullscreen_visual == VisualMode::Both),
        animation: !paused && !audio_busy,
        revision: 0,
        handled: engine.state.handled.len(),
        skipped: engine.state.skipped.len(),
        audio_busy,
        controlled_time: None,
    }
}
pub fn category(category: kavychka_core::reminders::Category, en: bool) -> &'static str {
    use kavychka_core::reminders::Category::*;
    match (en, category) {
        (false, Breakfast) => "Завтрак",
        (false, Lunch) => "Обед",
        (false, Dinner) => "Ужин",
        (false, Break) => "Перерыв",
        (false, Exercise) => "Разминка",
        (true, Breakfast) => "Breakfast",
        (true, Lunch) => "Lunch",
        (true, Dinner) => "Dinner",
        (true, Break) => "Break",
        (true, Exercise) => "Stretch",
    }
}

#[cfg(test)]
mod p4_policy_tests {
    use super::*;
    #[test]
    fn one_click_model_setup_preserves_unrelated_settings_and_rejects_stale_choices() {
        let base = Preferences::default();
        let mut current = base.clone();
        current.user_name = "Concurrent user name".into();
        current.generation.character_description = "Concurrent character".into();
        let mut desired = base.generation.clone();
        desired.text_enabled = true;
        desired.tts_model = "tts-cosyvoice3".into();
        desired.voice_profile = "voicefixture".into();
        desired.character_description = "Unrelated UI draft".into();
        let merged = merge_model_setup(&current, &base.generation, &desired).unwrap();
        assert!(merged.generation.text_enabled);
        assert_eq!(merged.generation.tts_model, "tts-cosyvoice3");
        assert_eq!(merged.generation.voice_profile, "voicefixture");
        assert_eq!(merged.user_name, "Concurrent user name");
        assert_eq!(
            merged.generation.character_description,
            "Concurrent character"
        );
        current.generation.tts_model = "tts-1.7b".into();
        assert!(merge_model_setup(&current, &base.generation, &desired).is_err());
    }
    #[test]
    fn a_manual_battery_exception_never_bypasses_pause_or_fullscreen_or_changes_the_preference() {
        let utc = "2026-10-23T12:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let clock = Clock {
            utc,
            mono: 1000,
            date: utc.date_naive(),
        };
        let mut engine = Engine::new(Preferences::default(), Default::default(), clock).unwrap();
        let battery = Environment {
            power: kavychka_core::engine::PowerSource::Battery,
            ..Default::default()
        };
        assert!(check_generation(&engine, clock, battery, false).is_err());
        assert!(check_generation(&engine, clock, battery, true).is_ok());
        assert!(engine.prefs.generation.ac_only);
        engine.pause(Some(utc + chrono::Duration::minutes(1)));
        assert!(check_generation(&engine, clock, battery, true).is_err());
        engine.pause(None);
        assert!(
            check_generation(
                &engine,
                clock,
                Environment {
                    fullscreen: true,
                    ..battery
                },
                true
            )
            .is_err()
        );
    }
}
