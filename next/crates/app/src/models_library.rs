//! Library orchestration on the service thread; inference stays in a child process.
use crate::inference::{self, Model, Worker};
use kavychka_core::{
    engine::{Clock, Effect, Engine, Environment, Notice},
    preferences::{Language, Preferences},
    reminders::Category,
};
use kavychka_storage::{library::Phrase, sandbox::Sandbox};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Serialize, serde::Deserialize)]
pub struct View {
    pub phrases: Vec<Phrase>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub page: usize,
    #[serde(default)]
    pub day: String,
    #[serde(default)]
    pub english: bool,
    #[serde(default)]
    pub time_zone: String,
    #[serde(default)]
    pub archived_pending: Vec<Notice>,
    #[serde(default)]
    pub archived_total: usize,
    #[serde(default)]
    pub archived_page: usize,
    pub models: Vec<Model>,
    pub status: String,
    pub running: bool,
    pub source: String,
    pub imported: usize,
    pub profiles: Vec<(String, String)>,
    #[serde(default)]
    pub presets: Vec<kavychka_storage::presets::Entry>,
}
struct Waiting {
    notice: Notice,
    deadline: chrono::DateTime<chrono::Utc>,
}
pub enum Preview {
    File(PathBuf),
    Text(String, Language),
}
pub struct Manager {
    app: PathBuf,
    resources: PathBuf,
    worker: Option<Worker>,
    kind: String,
    pub status: String,
    pub preview: Option<Preview>,
    waiting: Option<Waiting>,
    runtime_row: Option<Phrase>,
    runtime_request: Option<Value>,
    auto_attempts: u32,
    auto_date: String,
    auto_cancelled_day: Option<String>,
    last_publish: String,
    pub cancellation: Arc<AtomicBool>,
    attempted: std::collections::BTreeSet<String>,
    dirty: bool,
    preferred: Option<String>,
    auto_next: Option<chrono::DateTime<chrono::Utc>>,
    cleanup: Vec<PathBuf>,
    task_day: String,
    saved_preparation: String,
    runtime_deferred: bool,
    battery_override: bool,
    data_hold: bool,
    library_today: bool,
    library_page: usize,
    library_day: String,
    archive_page: usize,
    pub archived_count: usize,
}
impl Manager {
    pub(crate) fn set_measurement_resources(&mut self, path: PathBuf) {
        self.resources = path;
    }
    pub fn new(
        store: &mut Sandbox,
        source: Option<&Path>,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let restored_without_legacy = store.origin().map_err(|e| e.to_string())?.is_none()
            && store
                .meta("legacy_semantics_v2")
                .map_err(|e| e.to_string())?
                == Some(json!(true));
        if let Some(source) = source
            && !restored_without_legacy
        {
            store
                .bootstrap_legacy(source, &cancellation)
                .map_err(|e| e.to_string())?;
        }
        if store
            .meta("legacy_semantics_v2")
            .map_err(|e| e.to_string())?
            .is_none()
            && let Some(raw) = store.meta("legacy_settings").map_err(|e| e.to_string())?
        {
            let settings: kavychka_core::legacy::SettingsSnapshot =
                serde_json::from_value(raw).map_err(|e| e.to_string())?;
            let old = kavychka_core::legacy_active::preferences(&settings)?;
            let zone = old.desktop.time_zone;
            let now = kavychka_platform::time::now_in(&zone).map_err(|e| e.to_string())?;
            store
                .complete_legacy_semantics(now, |d, m| {
                    kavychka_platform::time::resolve_in(d, m, &zone).map_err(|e| e.to_string())
                })
                .map_err(|e| e.to_string())?;
        }
        store
            .apply_pending_recovery_limit()
            .map_err(|e| e.to_string())?;
        let resources = store
            .origin()
            .map_err(|e| e.to_string())?
            .map(|o| o.source)
            .unwrap_or_else(|| store.root.clone());
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let installed = executable.parent().ok_or("missing program folder")?;
        let app = if !(cfg!(test)
            || cfg!(debug_assertions)
                && executable
                    .file_name()
                    .is_some_and(|n| n == "kavychka-app.exe"))
        {
            installed.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .map_err(|e| e.to_string())?
        };
        let mut cleanup_warning = String::new();
        for job in std::fs::read_dir(store.root.join("jobs")).map_err(|e| e.to_string())? {
            let job = job.map_err(|e| e.to_string())?;
            if job.file_type().is_ok_and(|t| t.is_dir())
                && job.file_name().to_string_lossy().starts_with("job-")
                && let Err(error) = store.cleanup_staging(&job.path())
            {
                cleanup_warning = format!(
                    "Не удалось очистить временный кеш / Temporary cache cleanup failed: {error}"
                );
            }
        }
        let now = kavychka_platform::time::now_in(
            &store.load().map_err(|e| e.to_string())?.desktop.time_zone,
        )
        .map_err(|e| e.to_string())?;
        let saved = store
            .meta("preparation_state")
            .map_err(|e| e.to_string())?
            .or_else(|| {
                store
                    .meta("legacy_preparation_states")
                    .ok()
                    .flatten()
                    .and_then(|v| {
                        v.as_array().and_then(|a| {
                            a.iter().find(|v| v["day"] == now.date.to_string()).cloned()
                        })
                    })
            });
        let saved = saved.unwrap_or(json!({}));
        Ok(Self {
            app,
            resources,
            worker: None,
            kind: String::new(),
            status: cleanup_warning,
            preview: None,
            waiting: None,
            runtime_row: None,
            runtime_request: None,
            auto_attempts: saved["attempts"].as_u64().unwrap_or(0).min(3) as u32,
            auto_date: saved["day"].as_str().unwrap_or("").into(),
            auto_cancelled_day: saved["cancelled_day"].as_str().map(str::to_owned),
            last_publish: String::new(),
            cancellation,
            attempted: Default::default(),
            dirty: true,
            preferred: None,
            auto_next: saved["next"]
                .as_str()
                .map(str::parse)
                .transpose()
                .map_err(|_| "invalid saved retry timestamp")?,
            cleanup: vec![],
            task_day: String::new(),
            saved_preparation: String::new(),
            runtime_deferred: false,
            battery_override: false,
            data_hold: false,
            library_today: false,
            library_page: 0,
            library_day: now.date.to_string(),
            archive_page: 0,
            archived_count: 0,
        })
    }
    pub fn running(&self) -> bool {
        self.worker.is_some()
    }
    pub fn resume_preparation(&mut self) {
        self.auto_cancelled_day = None;
        self.auto_attempts = 0;
        self.auto_next = None;
    }
    fn take_user_cancellation(&mut self, clock: Clock) -> bool {
        let cancelled = self.cancellation.swap(false, Ordering::AcqRel);
        if cancelled {
            if self.kind == "daily" {
                self.auto_cancelled_day = Some(clock.date.to_string());
            }
            self.cancel();
            if let Some(wait) = &mut self.waiting {
                wait.deadline = wait.deadline.min(clock.utc);
            }
        }
        cancelled
    }
    fn persist_preparation(&mut self, store: &Sandbox) -> Result<(), String> {
        let value = json!({"day":self.auto_date,"attempts":self.auto_attempts,"next":self.auto_next,"cancelled_day":self.auto_cancelled_day});
        let saved = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        if self.saved_preparation != saved {
            store
                .put_meta("preparation_state", &value)
                .map_err(|e| e.to_string())?;
            self.saved_preparation = saved;
        }
        Ok(())
    }
    pub fn prepare_shutdown(&mut self, store: &Sandbox, clock: Clock) -> Result<(), String> {
        // Cancel immediately followed by Exit must survive the next launch.
        if self.take_user_cancellation(clock) {
            self.persist_preparation(store)?;
        }
        Ok(())
    }
    pub fn set_data_hold(&mut self, value: bool) {
        self.data_hold = value;
    }
    pub fn cancel(&mut self) {
        self.runtime_request = None;
        self.battery_override = false;
        self.cancellation.store(false, Ordering::Release);
        if let Some(worker) = &mut self.worker {
            worker.request_cancel();
        }
        self.status = "Генерация отменена / Generation cancelled".into();
        self.dirty = true;
    }
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn browse(&mut self, today: bool, page: usize) {
        self.library_today = today;
        self.library_page = page;
        self.invalidate();
    }
    pub fn browse_archive(&mut self, page: usize) {
        self.archive_page = page;
        self.invalidate();
    }
    pub fn fail(&mut self, error: String, clock: Clock) {
        let daily = self.kind == "daily";
        self.cancel();
        self.status = error;
        if daily {
            self.auto_attempts += 1;
            self.auto_next = Some(clock.utc + chrono::Duration::seconds(60));
        }
        if let Some(wait) = &mut self.waiting {
            wait.deadline = clock.utc;
        }
    }
    pub fn settings_changed(&mut self, clock: Clock) {
        if self.running() || self.waiting.is_some() {
            self.cancel();
        } else {
            self.battery_override = false;
        }
        self.runtime_row = None;
        self.preferred = None;
        if let Some(wait) = &mut self.waiting {
            wait.deadline = wait.deadline.min(clock.utc);
        }
        self.dirty = true;
        self.auto_attempts = 0;
        self.auto_next = None;
    }
    pub fn allow_current_task_on_battery(&mut self) {
        self.battery_override = self.worker.is_some() && self.kind == "batch";
    }
    pub fn preset_action(
        &mut self,
        store: &Sandbox,
        prefs: &Preferences,
        action: crate::presets_ui::Action,
    ) -> Result<(), String> {
        use kavychka_storage::presets::{self, Content, Preset};
        match action {
            crate::presets_ui::Action::Import(path, kind, transcript) => {
                if self.running() {
                    return Err("Отмени текущую задачу / Cancel the current task".into());
                }
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let id = if ext == "kavychka" || ext == "json" {
                    presets::import_file(&store.root, &path).map_err(|e| e.to_string())?
                } else if kind == "character" {
                    kavychka_storage::preview::no_link_chain(&path).map_err(|e| e.to_string())?;
                    if path.metadata().map_err(|e| e.to_string())?.len() > 20000 {
                        return Err("Описание слишком велико / Description is too large".into());
                    }
                    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                    presets::save_character(
                        &store.root,
                        "",
                        path.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .chars()
                            .take(80)
                            .collect(),
                        text.trim_start_matches('\u{feff}').trim().into(),
                    )
                    .map_err(|e| e.to_string())?
                } else if kind == "visual" {
                    crate::visuals::validate(&path, "auto")?;
                    let p = Preset {
                        version: 1,
                        name: path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .chars()
                            .take(80)
                            .collect(),
                        content: Content::Visual {
                            file: path
                                .file_name()
                                .ok_or("missing visual filename")?
                                .to_string_lossy()
                                .into(),
                            layout: "auto".into(),
                        },
                    };
                    presets::import_prepared(&store.root, &p, path.parent())
                        .map_err(|e| e.to_string())?
                } else if kind == "voice" && matches!(ext.as_str(), "wav" | "flac") {
                    let p = Preset {
                        version: 1,
                        name: path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .chars()
                            .take(80)
                            .collect(),
                        content: Content::Voice {
                            references: vec![presets::Reference {
                                file: path
                                    .file_name()
                                    .ok_or("missing audio name")?
                                    .to_string_lossy()
                                    .into(),
                                transcript,
                            }],
                        },
                    };
                    presets::import_prepared(&store.root, &p, path.parent())
                        .map_err(|e| e.to_string())?
                } else {
                    return self.start(store,json!({"operation":"preset_import","settings":prefs.generation.worker_settings(if prefs.speech_language==Language::Ru {"ru"}else{"en"}, &prefs.user_name),"input":path,"preset_kind":"voice","transcript":transcript}),"preset_import");
                };
                // Decode before exposing a visual in the selection lists.
                if let Content::Visual { file, layout } = presets::read(&store.root, &id)
                    .map_err(|e| e.to_string())?
                    .content
                    && let Err(error) = crate::visuals::validate(
                        &presets::inside(
                            &presets::folder(&store.root, &id).map_err(|e| e.to_string())?,
                            &file,
                        )
                        .map_err(|e| e.to_string())?,
                        &layout,
                    )
                {
                    presets::delete(&store.root, &id).map_err(|e| e.to_string())?;
                    return Err(error);
                }
                self.status = format!("Пресет добавлен / Preset imported: {id}");
            }
            crate::presets_ui::Action::Save(id, preset) => {
                if let Content::Visual { file, layout } = &preset.content {
                    crate::visuals::validate(
                        &presets::inside(
                            &presets::folder(&store.root, &id).map_err(|e| e.to_string())?,
                            file,
                        )
                        .map_err(|e| e.to_string())?,
                        layout,
                    )?;
                }
                if id.is_empty() {
                    if let Content::Character { description } = preset.content {
                        presets::save_character(&store.root, "", preset.name, description)
                            .map_err(|e| e.to_string())?;
                    } else {
                        return Err("Импортируй медиа сначала / Import media first".into());
                    }
                } else {
                    presets::update(&store.root, &id, &preset).map_err(|e| e.to_string())?;
                }
                self.status = "Пресет сохранён / Preset saved".into();
            }
            crate::presets_ui::Action::Delete(id) => {
                if id == prefs.generation.voice_profile
                    || id == prefs.desktop.character_preset
                    || id == prefs.desktop.visual_preset
                    || store.has_voice_reference(&id).map_err(|e| e.to_string())?
                {
                    return Err("Сначала выбери другой пресет и сохрани настройки / Select another preset and save first".into());
                }
                if self.running() {
                    return Err("Дождись задачи / Wait for the current task".into());
                }
                presets::delete(&store.root, &id).map_err(|e| e.to_string())?;
                self.status = "Пресет удалён / Preset deleted".into();
            }
            crate::presets_ui::Action::Export(id) => {
                let target = store.root.join("exports").join(format!(
                    "preset-{id}-{}.kavychka",
                    chrono::Utc::now().timestamp_millis()
                ));
                presets::export(&store.root, &id, &target).map_err(|e| e.to_string())?;
                self.status = format!("Пресет / Preset: {}", target.display());
            }
        }
        self.dirty = true;
        Ok(())
    }
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }
    fn start(&mut self, store: &Sandbox, mut request: Value, kind: &str) -> Result<(), String> {
        self.battery_override = false;
        if self.worker.is_some() {
            return Err(
                "Дождись текущей операции или отмени её / Cancel the current task first".into(),
            );
        }
        if let Some(rotation) = store.meta("voice_rotation").map_err(|e| e.to_string())? {
            request["rotation"] = rotation;
        }
        self.task_day = request["day"].as_str().unwrap_or("").into();
        self.worker = Some(Worker::start(
            &self.app,
            &store.root,
            &self.resources,
            request,
        )?);
        self.kind = kind.into();
        self.status = "Генерация… / Generating…".into();
        self.dirty = true;
        Ok(())
    }
    pub fn generate(
        &mut self,
        store: &mut Sandbox,
        prefs: &Preferences,
        clock: Clock,
        daily: bool,
    ) -> Result<(), String> {
        let rows = store.phrases().map_err(|e| e.to_string())?;
        let day = clock.date.to_string();
        let mut request = inference::request(prefs, &rows, "generate", &day);
        request["approved"] = json!(daily && prefs.generation.daily_auto_keep);
        let mut planning = Vec::new();
        if daily {
            for row in rows.iter().filter(|r| r.day.as_deref() == Some(&day)) {
                let manual = store
                    .meta(&format!("purpose:{}", row.id))
                    .map_err(|e| e.to_string())?
                    .is_some_and(|v| v == "batch");
                if !manual || row.approved || !prefs.generation.daily_auto_keep {
                    planning.push(row.clone());
                }
            }
        }
        if daily {
            let missing = planning
                .iter()
                .filter(|r| {
                    !r.deleted
                        && !r.rejected
                        && r.day.as_deref() == Some(&day)
                        && r.used_at.is_none()
                        && r.audio_path.is_none()
                        && r.model != "system"
                })
                .take(100)
                .cloned()
                .map(|mut r| {
                    r.approved |= prefs.generation.daily_auto_keep;
                    r
                })
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                request["operation"] = json!("speech");
                request["rows"] = json!(missing);
                return self.start(store, request, "daily");
            }
        }
        let tasks = if daily {
            daily_tasks(prefs, clock, &planning)?
        } else {
            batch_tasks(prefs)
        };
        if tasks.is_empty() {
            self.status = "День уже подготовлен / Day is ready".into();
            return Ok(());
        }
        let language = if prefs.speech_language == Language::Ru {
            "ru"
        } else {
            "en"
        };
        let mut known = rows
            .iter()
            .map(|r| kavychka_core::stock::normalize(&r.text))
            .collect::<std::collections::BTreeSet<_>>();
        let tasks = tasks
            .into_iter()
            .enumerate().map(|(i,(category, followup))|->Result<Value,String> {
                let mut task = json!({"category":category_name(category),"followup":followup});
                if !prefs.generation.text_enabled {
                    let seed=chrono::Datelike::num_days_from_ce(&clock.date) as u64+i as u64;
                    let text=kavychka_core::stock::candidates(language,category_name(category),followup,seed).find(|t|!known.contains(&kavychka_core::stock::normalize(t))).ok_or("Свежие встроенные варианты закончились. Можно включить модель текста / Enable a text model for more phrases")?;
                    known.insert(kavychka_core::stock::normalize(&text));task["text"]=json!(text);
                }
                Ok(task)
            })
            .collect::<Result<Vec<_>,_>>()?;
        if !prefs.generation.text_enabled && prefs.generation.tts_model == "system" {
            static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            for (i, task) in tasks.iter().enumerate() {
                let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
                let id = format!(
                    "stock-{:016x}{:016x}",
                    clock.utc.timestamp_millis() as u64,
                    ((std::process::id() as u64) << 32) | serial
                );
                let row = Phrase {
                    id,
                    text: task["text"].as_str().unwrap().into(),
                    category: task["category"].as_str().unwrap().into(),
                    model: "system".into(),
                    profile: "system".into(),
                    day: Some(day.clone()),
                    audio_path: None,
                    approved: daily && prefs.generation.daily_auto_keep,
                    deleted: false,
                    rejected: false,
                    used_at: None,
                    created_at: (clock.utc + chrono::Duration::nanoseconds(i as i64)).to_rfc3339(),
                    followup: task["followup"].as_bool().unwrap_or(false),
                    language: language.into(),
                    seed: 42,
                    voice_style: String::new(),
                    voice_instruction: String::new(),
                    voice_reference: String::new(),
                };
                store
                    .accept_phrase_and_rotation(
                        &row,
                        None,
                        None,
                        Some(if daily { "daily" } else { "batch" }),
                    )
                    .map_err(|e| e.to_string())?;
            }
            self.status = "Встроенные фразы подготовлены / Stock phrases prepared".into();
            self.dirty = true;
            return Ok(());
        }
        request["tasks"] = json!(tasks);
        self.start(store, request, if daily { "daily" } else { "batch" })
    }
    pub fn speech(
        &mut self,
        store: &mut Sandbox,
        prefs: &Preferences,
        clock: Clock,
        id: Option<&str>,
    ) -> Result<(), String> {
        let rows = store.phrases().map_err(|e| e.to_string())?;
        let selected = rows
            .iter()
            .filter(|r| {
                !r.deleted
                    && !r.rejected
                    && id.map_or(r.day.as_deref() == Some(&clock.date.to_string()), |id| {
                        r.id == id
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err("Нет выбранных фраз / No phrases selected".into());
        }
        if prefs.generation.tts_model == "system" {
            for mut row in selected {
                row.model = "system".into();
                row.profile = "system".into();
                row.audio_path = None;
                row.voice_style.clear();
                row.voice_instruction.clear();
                row.voice_reference.clear();
                store.accept_phrase(&row, None).map_err(|e| e.to_string())?;
            }
            self.status =
                "Выбран системный голос для сохранённых текстов / Saved texts use system speech"
                    .into();
            self.dirty = true;
            return Ok(());
        }
        let mut request = inference::request(prefs, &rows, "speech", &clock.date.to_string());
        request["rows"] = json!(selected);
        self.start(store, request, "speech")
    }
    pub fn export(
        &mut self,
        store: &Sandbox,
        prefs: &Preferences,
        clock: Clock,
    ) -> Result<(), String> {
        let rows = store.phrases().map_err(|e| e.to_string())?;
        let mut request = inference::request(prefs, &rows, "export", &clock.date.to_string());
        request["rows"] = json!(rows);
        self.start(store, request, "export")
    }
    pub fn install(
        &mut self,
        store: &Sandbox,
        prefs: &Preferences,
        clock: Clock,
    ) -> Result<(), String> {
        self.install_selected(store, prefs, clock, false)
    }
    pub fn repair_environment(
        &mut self,
        store: &Sandbox,
        prefs: &Preferences,
        clock: Clock,
    ) -> Result<(), String> {
        self.install_selected(store, prefs, clock, true)
    }
    fn install_selected(
        &mut self,
        store: &Sandbox,
        prefs: &Preferences,
        clock: Clock,
        repair: bool,
    ) -> Result<(), String> {
        let mut request = inference::request(prefs, &[], "install", &clock.date.to_string());
        request["repair_environment"] = json!(repair);
        let mut ids = vec![];
        if prefs.generation.text_enabled {
            ids.push(if prefs.generation.runtime {
                prefs.generation.runtime_text_model.clone()
            } else {
                prefs.generation.text_model.clone()
            });
        }
        if prefs.generation.tts_model != "system" {
            ids.push(prefs.generation.tts_model.clone());
        }
        if ids.is_empty() {
            self.status =
                "Базовый режим готов без загрузки / Basic mode is ready without downloads".into();
            self.dirty = true;
            return Ok(());
        }
        request["ids"] = json!(ids);
        self.start(store, request, "install")
    }
    pub fn tick(
        &mut self,
        store: &mut Sandbox,
        engine: &mut Engine,
        clock: Clock,
        env: Environment,
        effect: &mut Effect,
        live: bool,
    ) -> Result<(), String> {
        let day = clock.date.to_string();
        // Remember runtime attempts only while their notice is still active.
        // The text deduplication ledger is in SQLite and is not truncated here.
        if !self.attempted.is_empty() {
            let active: std::collections::BTreeSet<_> = engine
                .state
                .pending
                .iter()
                .map(|n| n.key.as_str())
                .collect();
            self.attempted.retain(|key| active.contains(key.as_str()));
        }
        if self.library_day != day {
            self.library_day = day;
            self.library_page = 0;
            self.invalidate();
        }
        if self.data_hold {
            self.runtime_deferred = true;
            return Ok(());
        }
        let new_day = clock.date.to_string();
        let fullscreen_deferred =
            env.fullscreen && engine.prefs.desktop.defer_generation_fullscreen;
        let battery_deferred =
            engine.prefs.generation.ac_only && env.power != kavychka_core::engine::PowerSource::Ac;
        let paused = engine.paused(clock);
        self.runtime_deferred = fullscreen_deferred || battery_deferred || paused;
        if self.running()
            && (fullscreen_deferred
                && ["daily", "batch", "runtime", "speech"].contains(&self.kind.as_str())
                || battery_deferred
                    && !self.battery_override
                    && ["daily", "batch", "runtime", "speech"].contains(&self.kind.as_str())
                || paused && ["daily", "batch", "runtime", "speech"].contains(&self.kind.as_str()))
        {
            self.cancel();
            if let Some(wait) = &mut self.waiting {
                wait.deadline = clock.utc;
            }
            self.status="Генерация отложена паузой, питанием или полным экраном / Generation deferred by pause, power or fullscreen settings".into();
        }
        if (self.running() || self.waiting.is_some())
            && !self.task_day.is_empty()
            && self.task_day < new_day
            && ["daily", "batch", "runtime", "speech"].contains(&self.kind.as_str())
        {
            self.cancel();
            self.waiting = None;
            self.runtime_row = None;
            self.status="Наступил новый день: прежняя генерация остановлена / Previous-day generation stopped".into();
        }
        for path in std::mem::take(&mut self.cleanup) {
            if let Err(error) = store.cleanup_staging(&path) {
                self.status = format!(
                    "Не удалось очистить временный кеш / Temporary cache cleanup failed: {error}"
                );
                self.dirty = true;
            }
        }
        let cancelled = self.take_user_cancellation(clock);
        let mut finished = false;
        let mut failed = false;
        if let Some(worker) = &mut self.worker {
            let was_cancelled = worker.cancelling();
            for result in worker.poll() {
                self.dirty = true;
                match result {
                    Err(error) => {
                        self.status = error;
                        finished = true;
                        failed = !was_cancelled;
                    }
                    Ok(value) => match value["kind"].as_str() {
                        Some("progress") => {
                            self.status = value["message"].as_str().unwrap_or("Генерация…").into();
                        }
                        Some("stage") => {
                            self.status = match value["stage"].as_str() {
                                Some("text") => "Генерация текста / Generating text",
                                Some("delivery") => "Подбор подачи / Choosing delivery",
                                Some("release_text") => {
                                    "Освобождение текстовой модели / Releasing text model"
                                }
                                Some("voice") => "Озвучка / Generating speech",
                                Some("publish") => "Проверка результата / Checking result",
                                _ => "Генерация… / Generating…",
                            }
                            .into();
                        }
                        Some("text") | Some("phrase") => {
                            let mut row: Phrase = serde_json::from_value(value["row"].clone())
                                .map_err(|e| e.to_string())?;
                            if value["kind"] == "text" {
                                row.approved = false;
                                row.audio_path = None;
                            }
                            store
                                .accept_phrase_and_rotation(
                                    &row,
                                    Some(&worker.root),
                                    value.get("rotation").and_then(Value::as_u64),
                                    if value["kind"] == "text" {
                                        Some(self.kind.as_str())
                                    } else {
                                        None
                                    },
                                )
                                .map_err(|e| e.to_string())?;
                            if self.kind == "runtime"
                                && value["kind"] == "phrase"
                                && self.waiting.is_some()
                            {
                                self.runtime_row = Some(row);
                            }
                        }
                        Some("result") => {
                            if self.kind == "preset_import" {
                                let id = value["preset"]["id"]
                                    .as_str()
                                    .ok_or("missing imported preset")?;
                                let p = kavychka_storage::presets::read(&worker.root, id)
                                    .map_err(|e| e.to_string())?;
                                kavychka_storage::presets::import_prepared(
                                    &store.root,
                                    &p,
                                    Some(
                                        &kavychka_storage::presets::folder(&worker.root, id)
                                            .map_err(|e| e.to_string())?,
                                    ),
                                )
                                .map_err(|e| e.to_string())?;
                                kavychka_storage::presets::delete(&worker.root, id)
                                    .map_err(|e| e.to_string())?;
                            }
                            if let Some(rotation) = value.get("rotation") {
                                store
                                    .put_meta("voice_rotation", rotation)
                                    .map_err(|e| e.to_string())?;
                            }
                            if self.kind == "export" {
                                let from = worker.root.join("library.zip");
                                let to = store.root.join("exports").join(format!(
                                    "{}-{}.zip",
                                    clock.date,
                                    clock.utc.timestamp_millis()
                                ));
                                std::fs::copy(from, &to).map_err(|e| e.to_string())?;
                                self.status = format!("ZIP: {}", to.display());
                            } else {
                                self.status = "Готово / Ready".into();
                            }
                            finished = true;
                        }
                        Some("error") => {
                            self.status = value["message"]
                                .as_str()
                                .unwrap_or("Ошибка генератора")
                                .into();
                            finished = true;
                            failed = true;
                        }
                        _ => {
                            self.status =
                                "Неверный ответ генератора / Invalid worker response".into();
                            finished = true;
                            failed = true;
                        }
                    },
                }
            }
            finished |= worker.done();
        }
        if finished {
            if let Some(worker) = self.worker.take() {
                self.battery_override = false;
                let path = worker.root.clone();
                drop(worker);
                if let Err(error) = store.cleanup_staging(&path) {
                    self.status = format!(
                        "Не удалось очистить временный кеш / Temporary cache cleanup failed: {error}"
                    );
                    self.dirty = true;
                }
            }
            if self.kind == "daily" {
                self.auto_next = Some(clock.utc + chrono::Duration::seconds(60));
                if failed && !cancelled {
                    self.auto_attempts += 1;
                }
            }
        }
        // Preemption waits for the retiring process tree without losing the
        // original event deadline. Never overlap heavy workers or restart a
        // request whose fallback has already been released.
        if !self.running() && self.runtime_request.is_some() {
            let eligible = self.waiting.as_ref().is_some_and(|wait| {
                clock.utc < wait.deadline
                    && engine
                        .state
                        .pending
                        .iter()
                        .any(|n| n.key == wait.notice.key && !n.visible)
            }) && !self.runtime_deferred
                && !env.session_blocked()
                && !env.notifications_blocked(engine.prefs.reminders.allow_during_dnd)
                && !(env.fullscreen
                    && engine.prefs.reminders.fullscreen_visual
                        == kavychka_core::preferences::VisualMode::Nothing
                    && engine.prefs.reminders.fullscreen_audio
                        == kavychka_core::preferences::AudioMode::Silent);
            let request = self.runtime_request.take().expect("queued request checked");
            if eligible {
                match self.start(store, request, "runtime") {
                    Ok(()) => finished = false,
                    Err(error) => {
                        self.status = error;
                        finished = true;
                    }
                }
            }
        }
        if let Some(wait) = &self.waiting {
            let reviewed = engine
                .state
                .pending
                .iter()
                .any(|n| n.key == wait.notice.key && n.visible);
            let blocked = env.session_blocked()
                || env.notifications_blocked(engine.prefs.reminders.allow_during_dnd)
                || engine.paused(clock)
                || (env.fullscreen
                    && engine.prefs.reminders.fullscreen_visual
                        == kavychka_core::preferences::VisualMode::Nothing
                    && engine.prefs.reminders.fullscreen_audio
                        == kavychka_core::preferences::AudioMode::Silent);
            if reviewed {
                // Explicit review has already delivered the fallback text quietly.
                self.waiting = None;
                self.runtime_row = None;
            } else if blocked
                || !engine
                    .state
                    .pending
                    .iter()
                    .any(|n| n.key == wait.notice.key)
            {
                self.waiting = None;
                self.runtime_row = None;
                if self.kind == "runtime" {
                    self.cancel();
                }
            } else if self.runtime_row.is_some() || clock.utc >= wait.deadline || finished {
                let mut notice = wait.notice.clone();
                if let Some(row) = self.runtime_row.take() {
                    notice.text = row.text;
                    self.preferred = Some(row.id);
                } else {
                    notice.text = kavychka_core::engine::builtin(
                        notice.events[0].category,
                        notice.events.len() > 1,
                        notice.followup,
                        engine.prefs.speech_language,
                    )
                    .into();
                }
                notice.visible = true;
                for pending in &mut engine.state.pending {
                    if pending.key == notice.key {
                        *pending = notice.clone();
                    }
                }
                effect.announce = Some(notice);
                self.waiting = None;
                // A late result is accepted only for future use, never spliced into delivered text.
            }
        }
        if self.waiting.is_none() {
            self.runtime_request = None;
        }
        let day = clock.date.to_string();
        if self.auto_date != day && !(self.kind == "export" && self.running()) {
            if live {
                store
                    .prune_quarantine(clock.date)
                    .map_err(|e| e.to_string())?;
            }
            self.auto_date = day.clone();
            self.auto_attempts = 0;
            self.auto_next = None;
            if let Some(old) = store
                .meta("audio_day")
                .map_err(|e| e.to_string())?
                .and_then(|v| v.as_str().map(str::to_owned))
                && old < day
            {
                store.expire_audio(&day, false).map_err(|e| e.to_string())?;
            }
            store
                .put_meta("audio_day", &json!(day))
                .map_err(|e| e.to_string())?;
        }
        let may_prepare = live
            && self.auto_cancelled_day.as_deref() != Some(&day)
            && !engine.prefs.generation.runtime
            && engine.prefs.generation.auto_daily
            && !engine.paused(clock)
            && (!env.fullscreen || !engine.prefs.desktop.defer_generation_fullscreen)
            && (!engine.prefs.generation.ac_only
                || env.power == kavychka_core::engine::PowerSource::Ac);
        if may_prepare
            && !self.running()
            && self.waiting.is_none()
            && self.auto_attempts < 3
            && self.auto_next.is_none_or(|at| at <= clock.utc)
        {
            self.auto_next = Some(clock.utc + chrono::Duration::seconds(60));
            if let Err(e) = self.generate(store, &engine.prefs, clock, true) {
                self.status = e;
                self.auto_attempts += 1;
                self.dirty = true;
            }
        }
        self.persist_preparation(store)?;
        Ok(())
    }
    /// Called before the reminder transaction; returns a phrase to claim atomically.
    pub fn deliver(
        &mut self,
        store: &Sandbox,
        engine: &mut Engine,
        clock: Clock,
        effect: &mut Effect,
    ) -> Result<Option<(String, String, Option<PathBuf>)>, String> {
        let Some(mut notice) = effect.announce.take() else {
            return Ok(None);
        };
        if self
            .waiting
            .as_ref()
            .is_some_and(|w| w.notice.key != notice.key)
        {
            self.waiting = None;
            self.runtime_row = None;
            if self.kind == "runtime" {
                self.cancel();
            }
        }
        let category = notice.events[0].category;
        if notice.events.len() > 1 {
            // Single-category recordings cannot speak for both actions. Keep the
            // complete combined fallback and use system speech; do not consume a
            // meal-only row or start a misleading single-category runtime task.
            self.preferred = None;
            notice.text = kavychka_core::engine::builtin(
                category,
                true,
                notice.followup,
                engine.prefs.speech_language,
            )
            .into();
            for pending in &mut engine.state.pending {
                if pending.key == notice.key {
                    pending.text = notice.text.clone();
                }
            }
            effect.announce = Some(notice);
            return Ok(None);
        }
        let anchor = notice.generation_due.unwrap_or_else(|| {
            if notice.followup > 0 {
                notice.created
            } else {
                notice
                    .events
                    .iter()
                    .map(|e| e.due)
                    .min()
                    .unwrap_or(notice.created)
            }
        });
        let deadline =
            anchor + chrono::Duration::seconds(engine.prefs.generation.runtime_wait_seconds.into());
        // Released runtime work already has an accepted candidate, so choose it like any ready phrase.
        if engine.prefs.generation.runtime
            && !self.runtime_deferred
            && deadline > clock.utc
            && self.waiting.is_none()
            && self.attempted.insert(notice.key.clone())
            && notice.text
                == kavychka_core::engine::builtin(
                    category,
                    notice.events.len() > 1,
                    notice.followup,
                    engine.prefs.speech_language,
                )
        {
            if self.running() {
                self.cancel();
            }
            let rows = store.phrases().map_err(|e| e.to_string())?;
            let mut request =
                inference::request(&engine.prefs, &rows, "generate", &clock.date.to_string());
            request["runtime"] = json!(true);
            request["approved"] = json!(true);
            request["tasks"] =
                json!([{"category":category_name(category),"followup":notice.followup>0}]);
            if !engine.prefs.generation.text_enabled {
                request["tasks"][0]["text"] = json!(notice.text);
            }
            let queued = self.running();
            let started = if queued {
                self.runtime_request = Some(request);
                true
            } else {
                self.start(store, request, "runtime").is_ok()
            };
            if started {
                for n in &mut engine.state.pending {
                    if n.key == notice.key {
                        n.visible = false;
                    }
                }
                self.waiting = Some(Waiting { notice, deadline });
                return Ok(None);
            }
        }
        let language = if engine.prefs.speech_language == Language::Ru {
            "ru"
        } else {
            "en"
        };
        let chosen = if let Some(id) = self.preferred.take() {
            Some(store.phrase(&id).map_err(|e| e.to_string())?)
        } else {
            store
                .choose(
                    &clock.date.to_string(),
                    category_name(category),
                    notice.followup > 0,
                    language,
                )
                .map_err(|e| e.to_string())?
        };
        let mut claim = None;
        if let Some(row) = chosen {
            let path = store.audio_file(&row).map_err(|e| e.to_string())?;
            notice.text = row.text;
            for n in &mut engine.state.pending {
                if n.key == notice.key {
                    n.text = notice.text.clone();
                }
            }
            claim = Some((notice.key.clone(), row.id, path));
            self.dirty = true;
        }
        effect.announce = Some(notice);
        Ok(claim)
    }
    pub fn view(&mut self, store: &Sandbox, prefs: &Preferences) -> Result<Option<String>, String> {
        if !self.dirty {
            return Ok(None);
        }
        self.dirty = false;
        let origin = store.origin().map_err(|e| e.to_string())?;
        let mut profiles = vec![(String::new(), "Кавычка · neutral".into())];
        let (presets, preset_warnings) =
            kavychka_storage::presets::scan(&store.root).map_err(|e| e.to_string())?;
        for entry in &presets {
            if matches!(
                entry.preset.content,
                kavychka_storage::presets::Content::Voice { .. }
            ) {
                profiles.push((entry.id.clone(), entry.preset.name.clone()));
            }
        }
        if !profiles
            .iter()
            .any(|(id, _)| id == &prefs.generation.voice_profile)
        {
            profiles.push((
                prefs.generation.voice_profile.clone(),
                format!("Недоступен / Missing · {}", prefs.generation.voice_profile),
            ));
        }
        let (phrases, total, page) = store
            .phrase_page(
                self.library_today.then_some(self.library_day.as_str()),
                self.library_page,
            )
            .map_err(|e| e.to_string())?;
        self.library_page = page;
        let (archived_pending, archived_total, archived_page) = store
            .pending_archive_page(self.archive_page)
            .map_err(|e| e.to_string())?;
        self.archive_page = archived_page;
        self.archived_count = archived_total;
        let mut status = self.status.clone();
        if self.auto_cancelled_day.as_deref() == Some(self.library_day.as_str()) {
            if !status.is_empty() {
                status.push('\n');
            }
            status.push_str(if prefs.ui_language == Language::En {
                "Automatic preparation is cancelled today. Use Prepare day to resume."
            } else {
                "Автоподготовка сегодня отменена. Возобновить — «Подготовить день»."
            });
        }
        let view = View {
            phrases,
            total,
            page,
            day: self.library_day.clone(),
            english: prefs.ui_language == Language::En,
            time_zone: prefs.desktop.time_zone.clone(),
            archived_pending,
            archived_total,
            archived_page,
            models: inference::models(&self.app, &self.resources, &store.root, prefs)?,
            status: if preset_warnings.is_empty() {
                status.clone()
            } else {
                format!(
                    "{}\nПресеты требуют проверки / Presets need review: {}",
                    status,
                    preset_warnings
                        .iter()
                        .take(10)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            },
            running: self.running(),
            source: origin
                .as_ref()
                .map(|o| o.source.display().to_string())
                .unwrap_or_default(),
            imported: origin.map(|o| o.imported_rows).unwrap_or(0),
            profiles,
            presets,
        };
        let json = serde_json::to_string(&view).map_err(|e| e.to_string())?;
        if json == self.last_publish {
            Ok(None)
        } else {
            self.last_publish = json.clone();
            Ok(Some(json))
        }
    }
}
pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Break => "break",
        Category::Breakfast => "breakfast",
        Category::Lunch => "lunch",
        Category::Dinner => "dinner",
        Category::Exercise => "exercise",
    }
}
fn batch_tasks(p: &Preferences) -> Vec<(Category, bool)> {
    let mut categories = vec![];
    for c in [
        Category::Breakfast,
        Category::Lunch,
        Category::Dinner,
        Category::Exercise,
    ] {
        if p.generation
            .categories
            .iter()
            .any(|s| s == category_name(c))
        {
            categories.push(c);
        }
    }
    let only = p
        .generation
        .categories
        .iter()
        .filter_map(|name| match name.as_str() {
            "break" => Some(Category::Break),
            "breakfast" => Some(Category::Breakfast),
            "lunch" => Some(Category::Lunch),
            "dinner" => Some(Category::Dinner),
            "exercise" => Some(Category::Exercise),
            _ => None,
        })
        .collect::<Vec<_>>();
    (0..p.generation.batch_count as usize)
        .map(|i| {
            (
                categories.get(i).copied().unwrap_or_else(|| {
                    if only.contains(&Category::Break) {
                        Category::Break
                    } else {
                        only[i % only.len()]
                    }
                }),
                false,
            )
        })
        .collect()
}
fn daily_tasks(
    p: &Preferences,
    clock: Clock,
    rows: &[Phrase],
) -> Result<Vec<(Category, bool)>, String> {
    let mut targets = std::collections::BTreeMap::<(String, bool), usize>::new();
    for event in kavychka_platform::time::day_in(&p.schedule, clock, &p.desktop.time_zone)
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|e| e.due >= clock.utc - chrono::Duration::minutes(5))
    {
        *targets
            .entry((category_name(event.category).into(), false))
            .or_default() += 1;
    }
    if p.schedule.exercise_enabled {
        targets.insert(("exercise".into(), false), 1);
    }
    for (category, n) in [
        ("break", p.reminders.break_repeats),
        ("exercise", p.reminders.exercise_repeats),
        ("breakfast", p.reminders.meal_repeats),
        ("lunch", p.reminders.meal_repeats),
        ("dinner", p.reminders.meal_repeats),
    ] {
        if targets.contains_key(&(category.into(), false)) {
            targets.insert((category.into(), true), n as usize);
        }
    }
    let day = clock.date.to_string();
    let language = if p.speech_language == Language::Ru {
        "ru"
    } else {
        "en"
    };
    for row in rows.iter().filter(|r| {
        !r.deleted
            && !r.rejected
            && r.used_at.is_none()
            && r.day.as_deref() == Some(&day)
            && r.language == language
    }) {
        if let Some(n) = targets.get_mut(&(row.category.clone(), row.followup)) {
            *n = n.saturating_sub(1);
        }
    }
    let mut result = vec![];
    for c in [
        Category::Breakfast,
        Category::Lunch,
        Category::Dinner,
        Category::Break,
        Category::Exercise,
    ] {
        for followup in [false, true] {
            for _ in 0..targets
                .get(&(category_name(c).into(), followup))
                .copied()
                .unwrap_or(0)
            {
                result.push((c, followup));
                if result.len() >= 100 {
                    return Ok(result);
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_attempts_are_retained_only_for_active_notices_not_entire_process_lifetime() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut engine = Engine::new(Preferences::default(), Default::default(), clock(0)).unwrap();
        let notice = engine.manual_stretch(clock(0)).announce.unwrap();
        manager
            .attempted
            .extend((0..10_000).map(|i| format!("completed-notice-{i}")));
        manager.attempted.insert(notice.key.clone());
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(1),
                Environment::default(),
                &mut Effect::default(),
                false,
            )
            .unwrap();
        assert_eq!(manager.attempted.len(), 1);
        assert!(manager.attempted.contains(&notice.key));
        engine.state.pending.clear();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(2),
                Environment::default(),
                &mut Effect::default(),
                false,
            )
            .unwrap();
        assert!(manager.attempted.is_empty());
    }
    #[test]
    fn cancellation_then_immediate_exit_is_preserved_but_normal_exit_is_not_cancel() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        manager.kind = "daily".into();
        manager.prepare_shutdown(&store, clock(0)).unwrap();
        assert!(store.meta("preparation_state").unwrap().is_none());
        manager.cancellation.store(true, Ordering::Release);
        manager.prepare_shutdown(&store, clock(0)).unwrap();
        drop(manager);
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.ac_only = false;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(62),
                Environment::default(),
                &mut Effect::default(),
                true,
            )
            .unwrap();
        assert!(store.phrases().unwrap().is_empty());
        let view: View =
            serde_json::from_str(&manager.view(&store, &engine.prefs).unwrap().unwrap()).unwrap();
        assert!(view.status.contains("Подготовить день"));
    }
    #[test]
    fn disabled_exercise_discards_waiting_and_late_runtime_replacement() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut engine = Engine::new(Preferences::default(), Default::default(), clock(0)).unwrap();
        let notice = engine.manual_stretch(clock(0)).announce.unwrap();
        manager.waiting = Some(Waiting {
            notice,
            deadline: clock(10).utc,
        });
        manager.runtime_row = Some(row("late"));
        manager.kind = "runtime".into();
        let mut prefs = engine.prefs.clone();
        prefs.schedule.exercise_enabled = false;
        engine.preferences(prefs, clock(1)).unwrap();
        manager.settings_changed(clock(1));
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(2),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(effect.announce.is_none());
        assert!(manager.waiting.is_none());
        assert!(manager.runtime_row.is_none());
        assert!(engine.state.pending.is_empty());
    }
    #[test]
    fn user_cancel_is_persisted_until_explicit_resume_or_next_day() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.ac_only = false;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        manager.kind = "daily".into();
        manager.cancellation.store(true, Ordering::Release);
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(0),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        assert!(store.phrases().unwrap().is_empty());
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(62),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        assert!(store.phrases().unwrap().is_empty());
        manager.resume_preparation();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(63),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        assert!(!store.phrases().unwrap().is_empty());
        manager.kind = "daily".into();
        manager.cancellation.store(true, Ordering::Release);
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(64),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(86400),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        assert!(
            store
                .phrases()
                .unwrap()
                .iter()
                .any(|r| r.day.as_deref() == Some(&clock(86400).date.to_string()))
        );
    }
    #[test]
    fn combined_notice_never_consumes_single_action_copy_or_runtime_work() {
        for runtime in [false, true] {
            let root = temp();
            let mut store = Sandbox::open(root.path()).unwrap();
            let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
            let mut meal = row("meal");
            meal.category = "lunch".into();
            meal.text = "Пообедай: суп остывает.".into();
            store.accept_phrase(&meal, None).unwrap();
            let mut p = Preferences::default();
            p.generation.runtime = runtime;
            let mut engine = Engine::new(p, Default::default(), clock(0)).unwrap();
            engine.schedule(
                vec![
                    kavychka_core::reminders::Event {
                        id: "meal".into(),
                        category: Category::Lunch,
                        due: clock(0).utc,
                        followup: false,
                    },
                    kavychka_core::reminders::Event {
                        id: "break".into(),
                        category: Category::Break,
                        due: clock(0).utc,
                        followup: false,
                    },
                ],
                clock(0),
                false,
            );
            let mut effect = engine.tick(clock(0), Environment::default(), false);
            let expected = effect.announce.as_ref().unwrap().text.clone();
            assert!(
                manager
                    .deliver(&store, &mut engine, clock(0), &mut effect)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(effect.announce.unwrap().text, expected);
            assert!(!manager.running());
            assert!(!manager.waiting());
            assert!(store.phrase("meal").unwrap().used_at.is_none());
        }
    }
    #[test]
    fn library_pages_follow_service_day_and_expose_rows_beyond_two_hundred() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        for i in 0..205 {
            let mut p = row(&format!("row{i:04}"));
            p.text = format!("Фраза {i}");
            store.accept_phrase(&p, None).unwrap();
        }
        let mut old = row("yesterday");
        old.day = Some((clock(0).date - chrono::Days::new(1)).to_string());
        store.accept_phrase(&old, None).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.auto_daily = false;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(0),
                Environment::default(),
                &mut Effect::default(),
                false,
            )
            .unwrap();
        manager.browse(true, 4);
        let view: View =
            serde_json::from_str(&manager.view(&store, &engine.prefs).unwrap().unwrap()).unwrap();
        assert_eq!(view.total, 205);
        assert_eq!(view.page, 4);
        assert_eq!(view.phrases.len(), 5);
        assert!(
            view.phrases
                .iter()
                .all(|p| p.day.as_deref() == Some(&view.day))
        );
        manager.browse(false, usize::MAX);
        let view: View =
            serde_json::from_str(&manager.view(&store, &engine.prefs).unwrap().unwrap()).unwrap();
        assert_eq!(view.total, 206);
        assert_eq!(view.phrases.len(), 6);
    }
    #[test]
    #[ignore = "explicit installed-model descriptor; no weights downloaded"]
    fn installed_models_daily_runtime_late_result_and_cancel() {
        use kavychka_core::engine::PowerSource;
        use std::time::{Duration, Instant};
        let descriptor =
            std::env::var_os("KAVYCHKA_P6_DESCRIPTOR").expect("Set an explicit private descriptor");
        let descriptor: Value =
            serde_json::from_slice(&std::fs::read(descriptor).unwrap()).unwrap();
        let evidence = PathBuf::from(descriptor["evidence"].as_str().unwrap());
        assert!(evidence.is_absolute() && !evidence.exists());
        std::fs::create_dir_all(&evidence).unwrap();
        let mut store = Sandbox::open(&evidence.join("data")).unwrap();
        let mut prefs: Preferences =
            serde_json::from_value(descriptor["preferences"].clone()).unwrap();
        let source = PathBuf::from(descriptor["preset_source"].as_str().unwrap());
        for p in kavychka_storage::presets::list(&source).unwrap() {
            let preset = kavychka_storage::presets::read(&source, &p.id).unwrap();
            kavychka_storage::presets::import_prepared(
                &store.root,
                &preset,
                Some(&kavychka_storage::presets::folder(&source, &p.id).unwrap()),
            )
            .unwrap();
        }
        // Imported presets use content-derived IDs; retain the selected voice by content.
        if !prefs.generation.voice_profile.is_empty() {
            let old =
                kavychka_storage::presets::read(&source, &prefs.generation.voice_profile).unwrap();
            prefs.generation.voice_profile = kavychka_storage::presets::import_prepared(
                &store.root,
                &old,
                Some(
                    &kavychka_storage::presets::folder(&source, &prefs.generation.voice_profile)
                        .unwrap(),
                ),
            )
            .unwrap();
        }
        prefs.generation.text_enabled = true;
        prefs.desktop.time_zone = "UTC".into();
        prefs.generation.auto_daily = false;
        prefs.generation.ac_only = true;
        prefs.generation.daily_auto_keep = true;
        prefs.generation.categories = vec!["break".into()];
        prefs.generation.batch_count = 1;
        prefs.reminders.break_repeats = 0;
        prefs.schedule.work_days = vec![6];
        prefs.schedule.work_start = 600;
        prefs.schedule.work_end = 610;
        prefs.schedule.break_every = 5;
        prefs.schedule.meals_enabled = false;
        prefs.schedule.exercise_enabled = false;
        store.save(&prefs).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        manager.resources = PathBuf::from(descriptor["resources"].as_str().unwrap());
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        let env = Environment {
            power: PowerSource::Ac,
            ..Default::default()
        };
        let mut effect = Effect::default();
        manager
            .generate(&mut store, &engine.prefs, clock(0), true)
            .unwrap();
        let start = Instant::now();
        while manager.running() {
            manager
                .tick(&mut store, &mut engine, clock(0), env, &mut effect, false)
                .unwrap();
            assert!(
                start.elapsed() < Duration::from_secs(900),
                "daily timeout: {}",
                manager.status
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let daily = store.phrases().unwrap();
        assert!(!daily.is_empty(), "{}", manager.status);
        assert!(daily.iter().all(|r| r.approved && r.audio_path.is_some()));
        for r in &daily {
            assert!(
                kavychka_platform::audio::decode_recording(
                    &store.audio_file(r).unwrap().unwrap(),
                    &|| false
                )
                .unwrap()
                .len()
                    > 8000
            );
        }
        let daily_count = daily.len();
        manager
            .generate(&mut store, &engine.prefs, clock(0), true)
            .unwrap();
        assert!(!manager.running());
        assert_eq!(store.phrases().unwrap().len(), daily_count);
        engine.prefs.generation.runtime = true;
        engine.prefs.generation.runtime_wait_seconds = 1;
        effect = engine.manual_stretch(clock(1));
        manager
            .deliver(&store, &mut engine, clock(1), &mut effect)
            .unwrap();
        assert!(manager.running() && manager.waiting());
        manager
            .tick(&mut store, &mut engine, clock(3), env, &mut effect, false)
            .unwrap();
        assert!(effect.announce.is_some() && !manager.waiting());
        let released = effect.announce.as_ref().unwrap().text.clone();
        manager
            .deliver(&store, &mut engine, clock(3), &mut effect)
            .unwrap();
        effect = Effect::default();
        while manager.running() {
            manager
                .tick(&mut store, &mut engine, clock(4), env, &mut effect, false)
                .unwrap();
            assert!(effect.announce.is_none());
            assert!(
                start.elapsed() < Duration::from_secs(1200),
                "runtime timeout: {}",
                manager.status
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(engine.state.pending.iter().any(|n| n.text == released));
        let rows = store.phrases().unwrap();
        assert_eq!(rows.len(), daily_count + 1, "{}", manager.status);
        assert!(
            rows.iter()
                .filter(|r| r.category == "exercise")
                .all(|r| r.audio_path.is_some() && r.used_at.is_none())
        );
        manager
            .generate(&mut store, &engine.prefs, clock(4), false)
            .unwrap();
        manager.cancel();
        let cancel = Instant::now();
        while manager.running() {
            manager
                .tick(&mut store, &mut engine, clock(5), env, &mut effect, false)
                .unwrap();
            assert!(cancel.elapsed() < Duration::from_secs(8));
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(store.phrases().unwrap().len(), rows.len());
        let cancel_millis = cancel.elapsed().as_millis();
        // A real background process must retire before the urgent process starts.
        engine = Engine::new(engine.prefs.clone(), Default::default(), clock(6)).unwrap();
        engine.prefs.generation.runtime_wait_seconds = 120;
        manager
            .generate(&mut store, &engine.prefs, clock(6), false)
            .unwrap();
        effect = engine.manual_stretch(clock(6));
        manager
            .deliver(&store, &mut engine, clock(6), &mut effect)
            .unwrap();
        assert!(
            manager.runtime_request.is_some() && manager.waiting() && effect.announce.is_none()
        );
        let preempt = Instant::now();
        while manager.kind != "runtime" {
            manager
                .tick(&mut store, &mut engine, clock(6), env, &mut effect, false)
                .unwrap();
            assert!(
                preempt.elapsed() < Duration::from_secs(8),
                "Urgent request was not started after retiring the background worker: {}",
                manager.status
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(manager.running() && manager.waiting() && manager.runtime_request.is_none());
        manager.cancellation.store(true, Ordering::Release);
        while manager.running() {
            manager
                .tick(&mut store, &mut engine, clock(7), env, &mut effect, false)
                .unwrap();
            assert!(preempt.elapsed() < Duration::from_secs(16));
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!manager.waiting() && manager.runtime_request.is_none());
        for job in std::fs::read_dir(store.root.join("jobs")).unwrap() {
            let job = job.unwrap().path();
            for name in [
                "audio",
                "payload.json",
                "cancel.request",
                "hf",
                "torch",
                "numba",
            ] {
                assert!(
                    !job.join(name).exists(),
                    "Temporary payload was retained: {name}"
                );
            }
        }
        std::fs::write(evidence.join("result.json"),serde_json::to_vec_pretty(&json!({"passed":true,"real_daily_rows":daily_count,"real_runtime_late_rows":1,"deadline_released_once":true,"late_result_no_replay":true,"daily_repeat_noop":true,"urgent_waited_for_retiring_background_process":true,"manual_cancel_released_queued_notice":true,"cancel_millis":cancel_millis,"weights_downloaded":false,"physical_events":false,"live_sqlite_opened":false,"elapsed_seconds":start.elapsed().as_secs()})).unwrap()).unwrap();
    }
    #[test]
    fn queued_runtime_past_deadline_releases_fallback_once_without_starting_a_child() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut p = store.load().unwrap();
        p.generation.runtime = true;
        p.generation.runtime_wait_seconds = 1;
        p.generation.auto_daily = false;
        let mut engine = Engine::new(p, Default::default(), clock(0)).unwrap();
        let effect = engine.manual_stretch(clock(0));
        let notice = effect.announce.unwrap();
        for n in &mut engine.state.pending {
            n.visible = false;
        }
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        manager.waiting = Some(Waiting {
            notice,
            deadline: clock(1).utc,
        });
        manager.runtime_request = Some(json!({"operation":"generate"}));
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(2),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(
            effect.announce.is_some()
                && !manager.waiting()
                && !manager.running()
                && manager.runtime_request.is_none()
        );
        assert_eq!(
            std::fs::read_dir(store.root.join("jobs")).unwrap().count(),
            0
        );
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(3),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(effect.announce.is_none());
    }
    #[test]
    fn restored_profile_startup_does_not_bootstrap_an_old_legacy_hint_again() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut p = store.load().unwrap();
        p.user_name = "Restored owner".into();
        p.desktop.setup_complete = true;
        store.save(&p).unwrap();
        store.put_meta("legacy_semantics_v2", &json!(true)).unwrap();
        let absent = root.path().join("old-source-no-longer-connected");
        let manager = Manager::new(&mut store, Some(&absent), Default::default()).unwrap();
        assert!(store.origin().unwrap().is_none());
        assert!(!root.path().join("legacy").exists());
        assert!(!absent.exists());
        assert!(store.phrases().unwrap().is_empty());
        assert_eq!(store.load().unwrap().user_name, "Restored owner");
        drop(manager);
    }
    fn clock(seconds: i64) -> Clock {
        let utc = "2026-10-24T09:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            + chrono::Duration::seconds(seconds);
        Clock {
            utc,
            mono: 1_000_000 + seconds as u64 * 1000,
            date: utc.date_naive(),
        }
    }
    fn temp() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/pipeline-tests");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn row(id: &str) -> Phrase {
        Phrase {
            id: id.into(),
            text: "Прервись и разомни плечи на минуту.".into(),
            category: "exercise".into(),
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
            language: "ru".into(),
            seed: 42,
            voice_style: String::new(),
            voice_instruction: String::new(),
            voice_reference: String::new(),
        }
    }
    #[test]
    fn runtime_deadline_releases_once_and_late_result_does_not_rewrite_delivered_text() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.runtime = true;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        let notice = engine.manual_stretch(clock(0)).announce.unwrap();
        engine.state.pending[0].visible = false;
        manager.attempted.insert(notice.key.clone());
        manager.waiting = Some(Waiting {
            notice: notice.clone(),
            deadline: clock(10).utc,
        });
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(9),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(effect.announce.is_none());
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(10),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(effect.announce.is_some());
        assert!(
            manager
                .deliver(&store, &mut engine, clock(10), &mut effect)
                .unwrap()
                .is_none()
        );
        assert!(!manager.waiting());
        let text = effect.announce.take().unwrap().text;
        let late = row("late");
        store.accept_phrase(&late, None).unwrap();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(11),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(effect.announce.is_none());
        assert_eq!(engine.state.pending[0].text, text);
        assert!(store.phrase("late").unwrap().used_at.is_none());
    }
    #[test]
    fn ready_phrase_is_claimed_only_with_reminder_and_never_selected_twice() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        store.accept_phrase(&row("ready"), None).unwrap();
        let mut engine = Engine::new(Preferences::default(), Default::default(), clock(0)).unwrap();
        let mut effect = engine.manual_stretch(clock(0));
        let claim = manager
            .deliver(&store, &mut engine, clock(0), &mut effect)
            .unwrap()
            .unwrap();
        assert_eq!(effect.announce.unwrap().text, row("ready").text);
        assert!(store.phrase("ready").unwrap().used_at.is_none());
        store
            .save_delivery(&engine.prefs, &engine.state, Some((&claim.0, &claim.1)))
            .unwrap();
        let mut next = engine.manual_stretch(clock(1));
        assert!(
            manager
                .deliver(&store, &mut engine, clock(1), &mut next)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn changed_language_releases_current_fallback_and_discards_old_runtime_candidate() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.runtime = true;
        let mut engine = Engine::new(prefs.clone(), Default::default(), clock(0)).unwrap();
        let notice = engine.manual_stretch(clock(0)).announce.unwrap();
        engine.state.pending[0].visible = false;
        manager.waiting = Some(Waiting {
            notice,
            deadline: clock(10).utc,
        });
        manager.runtime_row = Some(row("stale-russian-candidate"));
        manager.preferred = Some("stale-russian-candidate".into());
        manager
            .attempted
            .insert(engine.state.pending[0].key.clone());
        prefs.speech_language = Language::En;
        engine.preferences(prefs, clock(2)).unwrap();
        manager.settings_changed(clock(2));
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(2),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        let expected = kavychka_core::engine::builtin(Category::Exercise, false, 0, Language::En);
        assert_eq!(effect.announce.as_ref().unwrap().text, expected);
        assert!(
            manager
                .deliver(&store, &mut engine, clock(2), &mut effect)
                .unwrap()
                .is_none()
        );
        assert_eq!(engine.state.pending[0].text, expected);
        assert!(manager.runtime_row.is_none() && manager.preferred.is_none());
        assert!(!manager.running() && !manager.waiting());
        assert_eq!(
            std::fs::read_dir(root.path().join("jobs")).unwrap().count(),
            0
        );
    }
    #[test]
    fn battery_and_unknown_power_defer_daily_and_runtime_but_lock_allows_silent_preparation() {
        use kavychka_core::engine::PowerSource;
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.auto_daily = true;
        prefs.generation.ac_only = true;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        let mut effect = Effect::default();
        for power in [PowerSource::Battery, PowerSource::Unknown] {
            manager
                .tick(
                    &mut store,
                    &mut engine,
                    clock(0),
                    Environment {
                        power,
                        ..Default::default()
                    },
                    &mut effect,
                    true,
                )
                .unwrap();
            assert!(store.phrases().unwrap().is_empty());
            assert_eq!(manager.auto_attempts, 0);
            assert!(manager.runtime_deferred);
        }
        engine.prefs.generation.runtime = true;
        effect = engine.manual_stretch(clock(1));
        assert!(
            manager
                .deliver(&store, &mut engine, clock(1), &mut effect)
                .unwrap()
                .is_none()
        );
        assert!(effect.announce.is_some() && !manager.waiting() && !manager.running());
        engine.prefs.generation.runtime = false;
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(2),
                Environment {
                    locked: true,
                    ..Default::default()
                },
                &mut Effect::default(),
                true,
            )
            .unwrap();
        assert!(!store.phrases().unwrap().is_empty());
        assert!(!manager.running());
        assert_eq!(
            std::fs::read_dir(root.path().join("jobs")).unwrap().count(),
            0
        );
    }
    #[test]
    fn stock_day_preparation_and_system_voice_work_without_python_or_model_files() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.ac_only = false;
        manager
            .generate(&mut store, &prefs, clock(0), false)
            .unwrap();
        let rows = store.phrases().unwrap();
        assert_eq!(rows.len(), 13);
        assert!(rows.iter().all(|r| r.model == "system" && !r.approved));
        assert_eq!(
            rows.iter()
                .map(|r| kavychka_core::stock::normalize(&r.text))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            13
        );
        let original = rows[0].clone();
        manager
            .speech(&mut store, &prefs, clock(0), Some(&original.id))
            .unwrap();
        assert_eq!(store.phrase(&original.id).unwrap().text, original.text);
        assert!(!manager.running());
        assert_eq!(
            std::fs::read_dir(root.path().join("jobs")).unwrap().count(),
            0
        );
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(0),
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        let next = clock(86400);
        manager
            .tick(
                &mut store,
                &mut engine,
                next,
                Environment::default(),
                &mut effect,
                true,
            )
            .unwrap();
        let rows = store.phrases().unwrap();
        assert!(
            rows.iter()
                .any(|r| r.day.as_deref() == Some(&next.date.to_string()))
        );
        assert_eq!(
            rows.iter()
                .map(|r| kavychka_core::stock::normalize(&r.text))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            rows.len()
        );
        assert!(!manager.running());
    }
    #[test]
    fn queue_time_counts_against_the_runtime_budget() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut p = Preferences::default();
        p.generation.runtime = true;
        let mut engine = Engine::new(p, Default::default(), clock(0)).unwrap();
        engine.schedule(
            vec![kavychka_core::reminders::Event {
                id: "queued".into(),
                due: clock(0).utc,
                category: Category::Exercise,
                followup: false,
            }],
            clock(0),
            false,
        );
        assert!(
            engine
                .tick(clock(0), Environment::default(), true)
                .announce
                .is_none()
        );
        let mut effect = engine.tick(clock(20), Environment::default(), false);
        assert!(effect.announce.is_some());
        assert!(
            manager
                .deliver(&store, &mut engine, clock(20), &mut effect)
                .unwrap()
                .is_none()
        );
        assert!(!manager.waiting());
        assert!(!manager.running());
        assert_eq!(
            std::fs::read_dir(root.path().join("jobs")).unwrap().count(),
            0
        );
    }
    #[test]
    fn weekend_preparation_keeps_independent_exercise_and_manual_category_selection() {
        let prefs = Preferences::default();
        let tasks = daily_tasks(&prefs, clock(0), &[]).unwrap();
        assert_eq!(tasks.len(), 4);
        assert!(
            tasks
                .iter()
                .all(|(category, _)| *category == Category::Exercise)
        );
        let mut p = prefs;
        p.generation.categories = vec!["lunch".into()];
        p.generation.batch_count = 5;
        assert!(batch_tasks(&p).iter().all(|(c, _)| *c == Category::Lunch));
    }
    #[test]
    fn explicit_review_cancels_replacement_but_a_late_phrase_is_available_for_a_future_notice() {
        let root = temp();
        let mut store = Sandbox::open(root.path()).unwrap();
        let mut manager = Manager::new(&mut store, None, Default::default()).unwrap();
        let mut prefs = Preferences::default();
        prefs.generation.runtime = true;
        let mut engine = Engine::new(prefs, Default::default(), clock(0)).unwrap();
        let original = engine.manual_stretch(clock(0)).announce.unwrap();
        engine.state.pending[0].visible = false;
        manager.attempted.insert(original.key.clone());
        manager.waiting = Some(Waiting {
            notice: original.clone(),
            deadline: clock(10).utc,
        });
        engine.review();
        let mut effect = Effect::default();
        manager
            .tick(
                &mut store,
                &mut engine,
                clock(1),
                Environment::default(),
                &mut effect,
                false,
            )
            .unwrap();
        assert!(!manager.waiting());
        assert!(effect.announce.is_none());
        assert_eq!(engine.state.pending[0].text, original.text);
        store.accept_phrase(&row("late"), None).unwrap();
        let mut future = engine.manual_stretch(clock(2));
        let claim = manager
            .deliver(&store, &mut engine, clock(2), &mut future)
            .unwrap()
            .unwrap();
        assert_eq!(claim.1, "late");
        assert!(store.phrase("late").unwrap().used_at.is_none());
    }
}
