//! One cancellable data operation at a time. Workers read/copy private snapshots;
//! only the service applies a prepared plan to the working database.
use crate::{
    AppWindow,
    service::{Backend, Command},
};
use kavychka_storage::{
    profile::{
        self, ApplyReport, CaptureSource, ExportOptions, ExportReport, RestorePlan, RestoreSummary,
    },
    sandbox::Sandbox,
};
use serde::Serialize;
use slint::ComponentHandle;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};
pub enum Action {
    Diagnose,
    ExportDiagnostics,
    Estimate(ExportOptions),
    Export(PathBuf, ExportOptions),
    Inspect(PathBuf),
    Apply,
    Cancel,
    Discard,
}
enum Event {
    DiagnosticExported(Result<PathBuf, String>),
    Failed(String),
    Progress(String),
    Estimated(Result<(ExportOptions, u64, u64), String>),
    Exported(Result<ExportReport, String>),
    Inspected(Result<RestorePlan, String>),
    Prepared(Result<RestorePlan, String>),
}
#[derive(Default, Clone, Serialize)]
struct View {
    busy: bool,
    review: String,
    can_apply: bool,
    status_ru: String,
    status_en: String,
    summary_ru: String,
    summary_en: String,
    output: String,
}
pub struct Manager {
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    join: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    view: View,
    last: String,
    plan: Option<RestorePlan>,
    ready: Option<RestorePlan>,
    estimate: Option<ExportOptions>,
    diagnostic: Option<kavychka_maintenance::diagnostics::Report>,
}
impl Manager {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            join: None,
            cancel: Arc::new(AtomicBool::new(false)),
            view: View::default(),
            last: String::new(),
            plan: None,
            ready: None,
            estimate: None,
            diagnostic: None,
        }
    }
    pub fn blocked(&self) -> bool {
        self.view.busy || !self.view.review.is_empty()
    }
    fn status(&mut self, ru: impl Into<String>, en: impl Into<String>) {
        self.view.status_ru = ru.into();
        self.view.status_en = en.into();
    }
    pub fn fail(&mut self, error: String) {
        self.status(
            format!("Операция с данными не выполнена: {error}"),
            format!("Data operation failed: {error}"),
        );
    }
    fn worker(
        &mut self,
        work: impl FnOnce(mpsc::Sender<Event>, Arc<AtomicBool>) + Send + 'static,
    ) -> Result<(), String> {
        if self.join.is_some() {
            return Err(
                "Дождись завершения текущей операции / Wait for the current operation".into(),
            );
        }
        self.cancel.store(false, Ordering::Release);
        self.view.busy = true;
        self.view.can_apply = false;
        let sender = self.sender.clone();
        let token = self.cancel.clone();
        self.join = Some(std::thread::spawn(move || {
            let failure = sender.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(sender, token)))
                .is_err()
            {
                let _ = failure.send(Event::Failed(
                    "Data worker failed; current profile retained".into(),
                ));
            }
        }));
        Ok(())
    }
    pub fn action(
        &mut self,
        action: Action,
        store: &Sandbox,
        generator_busy: bool,
    ) -> Result<(), String> {
        if matches!(action, Action::Cancel | Action::Discard) {
            self.cancel.store(true, Ordering::Release);
            self.plan = None;
            self.ready = None;
            self.estimate = None;
            self.diagnostic = None;
            self.view.review.clear();
            self.view.can_apply = false;
            self.status("Отмена операции…", "Cancelling…");
            if self.join.is_none() {
                self.view.busy = false;
                self.status("Операция отменена", "Operation cancelled");
            }
            return Ok(());
        }
        if self.view.busy || generator_busy {
            return Err("Сначала дождись или отмени текущую задачу / Finish or cancel the current task first".into());
        }
        let source = CaptureSource::new(store).map_err(|e| e.to_string())?;
        let root = store.root.clone();
        let version = crate::app_version().to_owned();
        match action {
            Action::Diagnose => {
                let report = kavychka_maintenance::diagnostics::capture(store, &version)
                    .map_err(|e| e.to_string())?;
                self.view.summary_ru = format!(
                    "Версия: {}. Фраз: {}, записей: {}, исключённых: {}.\n{}\n\nЭто полный предварительный просмотр JSON. Тексты, имена, описания, пути, референсы и журналы не включены. Ничего не отправляется.\n{}",
                    version,
                    report.rows,
                    report.recordings,
                    report.excluded,
                    report
                        .checks
                        .iter()
                        .map(|c| format!("{}: {}", c.cause_ru, c.action_ru))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                );
                self.view.summary_en = format!(
                    "Version: {}. Phrases: {}, recordings: {}, excluded: {}.\n{}\n\nFull JSON preview below. No texts, names, prompts, paths, references or logs. Nothing is sent.\n{}",
                    version,
                    report.rows,
                    report.recordings,
                    report.excluded,
                    report
                        .checks
                        .iter()
                        .map(|c| format!("{}: {}", c.cause_en, c.action_en))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                );
                self.diagnostic = Some(report);
                self.view.review = "diagnostic".into();
                self.status(
                    "Диагностика готова к просмотру",
                    "Diagnostics ready for review",
                );
            }
            Action::ExportDiagnostics => {
                let report = self.diagnostic.take().ok_or("Preview diagnostics first")?;
                self.view.review.clear();
                self.worker(move |tx, _| {
                    let output = root
                        .join("exports")
                        .join(format!("diagnostics-{}.json", id()));
                    let result = kavychka_maintenance::diagnostics::export(&report, &output)
                        .map(|()| output)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::DiagnosticExported(result));
                })?;
            }
            Action::Estimate(options) => {
                self.plan = None;
                self.ready = None;
                self.view.review.clear();
                self.status("Считаю состав и место…", "Estimating contents and space…");
                self.worker(move |tx, cancel| {
                    let result = source
                        .estimate(options, &cancel)
                        .map(|(bytes, peak)| (options, bytes, peak))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Estimated(result));
                })?;
            }
            Action::Export(output, options) => {
                if self.estimate != Some(options) {
                    return Err(
                        "Сначала рассчитай выбранный состав / Estimate the selected contents first"
                            .into(),
                    );
                }
                self.view.review.clear();
                self.status("Готовлю архив профиля…", "Preparing profile archive…");
                self.worker(move |tx, cancel| {
                    let job = root.join("jobs").join(format!("profile-export-{}", id()));
                    let progress = tx.clone();
                    let result = source
                        .export(&output, &job, options, &version, &cancel, move |stage| {
                            let _ = progress.send(Event::Progress(stage.into()));
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Exported(result));
                })?;
            }
            Action::Inspect(archive) => {
                self.plan = None;
                self.ready = None;
                self.view.review.clear();
                self.status(
                    "Проверяю архив; данные пока не меняются…",
                    "Inspecting archive; current data is unchanged…",
                );
                let limit = store
                    .load()
                    .map_err(|e| e.to_string())?
                    .desktop
                    .recovery_limit_bytes;
                self.worker(move |tx, cancel| {
                    let progress = tx.clone();
                    let result = profile::inspect(
                        &archive,
                        &root.join("jobs").join(format!("profile-inspect-{}", id())),
                        limit,
                        &cancel,
                        move |stage| {
                            let _ = progress.send(Event::Progress(stage.into()));
                        },
                    )
                    .and_then(|inspected| {
                        let prefs = inspected.preferences();
                        let clock = kavychka_platform::time::now_in(&prefs.desktop.time_zone)
                            .map_err(|e| kavychka_storage::Error::Invalid(e.to_string()))?;
                        kavychka_platform::time::day_in(
                            &prefs.schedule,
                            clock,
                            &prefs.desktop.time_zone,
                        )
                        .map_err(|e| kavychka_storage::Error::Invalid(e.to_string()))?;
                        source.prepare_restore(inspected, &cancel)
                    })
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Inspected(result));
                })?;
            }
            Action::Apply => {
                let mut plan = self
                    .plan
                    .take()
                    .ok_or("Сначала проверь архив / Inspect an archive first")?;
                self.view.review.clear();
                self.status(
                    "Создаю проверенный резерв и готовлю файлы…",
                    "Creating a verified backup and preparing files…",
                );
                self.worker(move |tx, cancel| {
                    let progress = tx.clone();
                    let result = plan
                        .prepare_files(source, &version, &cancel, move |stage| {
                            let _ = progress.send(Event::Progress(stage.into()));
                        })
                        .map(|()| plan)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Prepared(result));
                })?;
            }
            Action::Cancel | Action::Discard => unreachable!(),
        }
        Ok(())
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            if let Event::Progress(stage) = event {
                if self.cancel.load(Ordering::Acquire) {
                    continue;
                }
                let (ru, en) = match stage.as_str() {
                    "snapshot" => (
                        "Создаю согласованный снимок SQLite…",
                        "Capturing a consistent SQLite snapshot…",
                    ),
                    "files" => (
                        "Копирую данные; модели исключены…",
                        "Copying data; models excluded…",
                    ),
                    "archive" | "archive-writing" => {
                        ("Собираю и перепроверяю ZIP…", "Writing and verifying ZIP…")
                    }
                    "verify" => (
                        "Проверяю хеши, схему и ссылки…",
                        "Checking hashes, schema and references…",
                    ),
                    "backup" => (
                        "Создаю резерв перед восстановлением…",
                        "Creating pre-restore backup…",
                    ),
                    "prepare" => (
                        "Готовлю новые файлы отдельно…",
                        "Preparing new files separately…",
                    ),
                    _ => ("Завершаю проверку…", "Finishing verification…"),
                };
                self.status(ru, en);
                continue;
            }
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
            if self.cancel.load(Ordering::Acquire)
                && !matches!(
                    &event,
                    Event::Exported(Ok(_)) | Event::DiagnosticExported(Ok(_))
                )
            {
                self.view.busy = false;
                self.view.review.clear();
                self.status(
                    "Операция отменена; текущие данные сохранены",
                    "Cancelled; current data retained",
                );
                continue;
            }
            self.view.busy = false;
            match event {
                Event::DiagnosticExported(Ok(path)) => {
                    self.view.output = path.display().to_string();
                    self.status(
                        "Диагностика сохранена локально; ничего не отправлено",
                        "Diagnostics saved locally; nothing sent",
                    );
                }
                Event::Estimated(Ok((options, bytes, peak))) => {
                    self.estimate = Some(options);
                    self.view.review = "export".into();
                    self.view.summary_ru = format!(
                        "Данные: примерно {:.1} МБ. Временный пик: до {:.1} МБ. Настройки и пресеты включены. Библиотека и история: {}. Сгенерированные записи: {}. Веса, среды, код и частные логи исключены.",
                        mb(bytes),
                        mb(peak),
                        yes(options.library, false),
                        yes(options.library && options.audio, false)
                    );
                    self.view.summary_en = format!(
                        "Data: about {:.1} MB. Temporary peak: up to {:.1} MB. Settings and presets included. Library and history: {}. Generated recordings: {}. Weights, runtimes, code and private logs excluded.",
                        mb(bytes),
                        mb(peak),
                        yes(options.library, true),
                        yes(options.library && options.audio, true)
                    );
                    self.status(
                        "Состав рассчитан; выбери новый ZIP и экспортируй",
                        "Contents estimated; choose a new ZIP and export",
                    );
                }
                Event::Exported(Ok(report)) => {
                    self.estimate = None;
                    self.view.output = report.path.display().to_string();
                    self.status(
                        format!("Архив проверен и сохранён: {}", report.path.display()),
                        format!("Archive verified and saved: {}", report.path.display()),
                    );
                }
                Event::Inspected(Ok(plan)) => {
                    self.view.review = "restore".into();
                    self.view.can_apply = true;
                    self.describe(plan.summary());
                    self.plan = Some(plan);
                    self.status(
                        "Проверь сводку перед применением",
                        "Review the summary before applying",
                    );
                }
                Event::Prepared(Ok(plan)) => {
                    self.view.busy = true;
                    self.ready = Some(plan);
                }
                Event::Failed(e)
                | Event::DiagnosticExported(Err(e))
                | Event::Estimated(Err(e))
                | Event::Exported(Err(e))
                | Event::Inspected(Err(e))
                | Event::Prepared(Err(e)) => {
                    self.view.review.clear();
                    self.fail(e);
                }
                Event::Progress(_) => unreachable!(),
            }
        }
    }
    fn describe(&mut self, s: &RestoreSummary) {
        let conflicts = s
            .conflicts
            .iter()
            .map(|c| format!("{} · {}: {}", c.kind, c.id, c.resolution))
            .collect::<Vec<_>>()
            .join("\n");
        self.view.summary_ru = format!(
            "{}\nДобавится фраз: {}. Текущих сохранится: {}. Совпадений текста: {}. Пресетов: +{}, повторно используется {}. Новых файлов: {}, {:.1} МБ. Конфликтов: {}.\nРезерв: примерно {:.1} МБ; временный пик: {:.1} МБ. Недоступных путей моделей: {}. Пути из архива не становятся путями записи.\n{}\n{}",
            if s.clean_profile {
                "Чистая установка: восстановятся настройки и состояние; лимит резервов этой установки сохранится."
            } else {
                "Текущие настройки, состояние и исключения сохранятся."
            },
            s.phrases_added,
            s.phrases_kept,
            s.duplicate_texts,
            s.presets_added,
            s.presets_reused,
            s.files_added,
            mb(s.bytes_added),
            s.conflict_count,
            mb(s.backup_estimate_bytes),
            mb(s.peak_bytes),
            s.unavailable_model_paths.len(),
            if s.already_imported {
                "Этот архив уже применён; повтор будет без изменений."
            } else {
                "Перед применением будет создан проверенный резерв. Голос автоматически не запускается."
            },
            conflicts
        );
        self.view.summary_en = format!(
            "{}\nPhrases added: {}. Current rows kept: {}. Duplicate texts: {}. Presets: +{}, reused {}. New files: {}, {:.1} MB. Conflicts: {}.\nBackup: about {:.1} MB; temporary peak: {:.1} MB. Unavailable model paths: {}. Archive paths are never write destinations.\n{}\n{}",
            if s.clean_profile {
                "Clean profile: settings and state restored; this installation's recovery budget retained."
            } else {
                "Current settings, state and exclusions will be kept."
            },
            s.phrases_added,
            s.phrases_kept,
            s.duplicate_texts,
            s.presets_added,
            s.presets_reused,
            s.files_added,
            mb(s.bytes_added),
            s.conflict_count,
            mb(s.backup_estimate_bytes),
            mb(s.peak_bytes),
            s.unavailable_model_paths.len(),
            if s.already_imported {
                "This archive was already applied; repeating makes no changes."
            } else {
                "A verified backup will be created before applying. Voice never starts automatically."
            },
            conflicts
        );
    }
    pub fn take_ready(&mut self) -> Option<RestorePlan> {
        self.ready.take()
    }
    pub fn applied(&mut self, result: Result<ApplyReport, String>) {
        self.view.busy = false;
        self.view.review.clear();
        self.view.can_apply = false;
        self.plan = None;
        self.estimate = None;
        match result {
            Ok(report) => {
                self.status(
                    format!(
                        "Восстановление завершено. Добавлено фраз: {}, пресетов: {}. {}",
                        report.phrases_added,
                        report.presets_added,
                        report.warning.as_deref().unwrap_or("")
                    ),
                    format!(
                        "Restore complete. Phrases added: {}, presets: {}. {}",
                        report.phrases_added,
                        report.presets_added,
                        report.warning.as_deref().unwrap_or("")
                    ),
                );
                self.view.output = report
                    .backup
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
            Err(error) => self.fail(error),
        }
    }
    pub fn publish(&mut self, ui: &slint::Weak<AppWindow>) {
        let json = serde_json::to_string(&self.view).expect("serializable data view");
        if json == self.last {
            return;
        }
        self.last = json;
        let view = self.view.clone();
        let active = self.blocked();
        let _ = ui.upgrade_in_event_loop(move |ui| {
            ui.set_data_operation_active(active);
            ui.set_profile_running(view.busy);
            ui.set_profile_review(view.review.into());
            ui.set_profile_can_apply(view.can_apply);
            ui.set_profile_status_ru(view.status_ru.into());
            ui.set_profile_status_en(view.status_en.into());
            ui.set_profile_summary_ru(view.summary_ru.into());
            ui.set_profile_summary_en(view.summary_en.into());
            ui.set_profile_output(view.output.into());
        });
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn id() -> String {
    format!(
        "{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.
}
fn yes(value: bool, en: bool) -> &'static str {
    match (value, en) {
        (true, false) => "да",
        (false, false) => "нет",
        (true, true) => "yes",
        (false, true) => "no",
    }
}
pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    ui.on_profile_browse(move |save| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if ui.get_profile_running() {
            return;
        }
        let initial = if save {
            ui.get_profile_export_path()
        } else {
            ui.get_profile_import_path()
        };
        match kavychka_platform::dialogs::profile_zip(
            save,
            initial.as_str(),
            crate::shell::handle(ui.window()),
        ) {
            Ok(Some(path)) => {
                if save {
                    ui.set_profile_export_path(path.to_string_lossy().as_ref().into());
                } else {
                    ui.set_profile_import_path(path.to_string_lossy().as_ref().into());
                    if ui.get_onboarding() {
                        ui.invoke_profile_action(2);
                    }
                }
            }
            Ok(None) => (),
            Err(error) => {
                ui.set_profile_status_ru(error.clone().into());
                ui.set_profile_status_en(error.into());
            }
        }
    });
    let sender = backend.sender.clone();
    let weak = ui.as_weak();
    ui.on_profile_action(move |kind| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if ui.get_dirty() && !ui.get_onboarding() && kind != 4 && kind != 5 {
            ui.set_profile_status_ru("Сначала сохрани или отмени изменения настроек".into());
            ui.set_profile_status_en("Save or discard settings changes first".into());
            return;
        }
        let options = ExportOptions {
            library: ui.get_profile_include_library(),
            audio: ui.get_profile_include_audio(),
        };
        let action = match kind {
            0 => Action::Estimate(options),
            1 => Action::Export(
                PathBuf::from(ui.get_profile_export_path().as_str()),
                options,
            ),
            2 => Action::Inspect(PathBuf::from(ui.get_profile_import_path().as_str())),
            3 => Action::Apply,
            4 => Action::Cancel,
            6 => Action::Diagnose,
            7 => Action::ExportDiagnostics,
            _ => Action::Discard,
        };
        if let Err(error) = sender.try_send(Command::Profile(action)) {
            ui.set_profile_status_ru(error.to_string().into());
            ui.set_profile_status_en(error.to_string().into());
        }
    });
}
