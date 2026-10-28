//! Preview in the settings window; offline copy only after the service has exited.
use crate::{AppWindow, TransferWindow};
use kavychka_maintenance::{Result, install, relocate};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub struct Controller {
    root: Option<PathBuf>,
    restart: Rc<RefCell<Option<PathBuf>>>,
    cancel: Arc<AtomicBool>,
    join: Rc<RefCell<Option<std::thread::JoinHandle<()>>>>,
    _timer: slint::Timer,
}
impl Controller {
    pub fn new(ui: &AppWindow) -> Self {
        let root = std::env::var_os("KAVYCHKA_INSTALL_ROOT")
            .filter(|_| std::env::var_os("KAVYCHKA_PORTABLE_ROOT").is_none())
            .map(PathBuf::from)
            .filter(|root| {
                install::current(root).is_ok_and(|c| {
                    c.is_some_and(|c| c.data == Path::new(ui.get_storage_path().as_str()))
                })
            });
        Self::for_root(ui, root)
    }
    fn for_root(ui: &AppWindow, root: Option<PathBuf>) -> Self {
        ui.set_transfer_available(root.is_some());
        let cancel = Arc::new(AtomicBool::new(false));
        let join = Rc::new(RefCell::new(None::<std::thread::JoinHandle<()>>));
        let restart = Rc::new(RefCell::new(None));
        let estimate = Rc::new(RefCell::new(None::<relocate::Estimate>));
        let (tx, rx) = mpsc::channel();
        let weak = ui.as_weak();
        let r = root.clone();
        let token = cancel.clone();
        let j = join.clone();
        let txc = tx.clone();
        let e = estimate.clone();
        ui.on_transfer_estimate(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(root) = r.clone() else {
                return;
            };
            if j.borrow().is_some()
                || ui.get_dirty()
                || ui.get_data_operation_active()
                || ui.get_update_running()
            {
                return;
            }
            let Ok(prefs) = serde_json::from_str::<kavychka_core::preferences::Preferences>(
                ui.get_loaded_prefs().as_str(),
            ) else {
                ui.set_transfer_status(
                    "Дождись загрузки настроек / Wait for settings to load".into(),
                );
                return;
            };
            let models = prefs.generation.model_paths;
            *e.borrow_mut() = None;
            ui.set_transfer_reviewed(false);
            ui.set_transfer_busy(true);
            ui.set_transfer_status(
                "Считаю состав и свободное место… / Estimating contents and disk space…".into(),
            );
            token.store(false, Ordering::Release);
            let cancel = token.clone();
            let tx = txc.clone();
            let target = PathBuf::from(ui.get_transfer_target().as_str());
            *j.borrow_mut() = Some(std::thread::spawn(move || {
                let result = std::panic::catch_unwind(|| {
                    relocate::estimate_with_models(&root, &target, &cancel, &models)
                        .map_err(|e| e.to_string())
                })
                .unwrap_or_else(|_| Err("Storage preview worker failed".into()));
                let _ = tx.send(result);
            }));
        });
        let weak = ui.as_weak();
        let token = cancel.clone();
        let e = estimate.clone();
        ui.on_transfer_cancel(move || {
            token.store(true, Ordering::Release);
            *e.borrow_mut() = None;
            if let Some(ui) = weak.upgrade() {
                ui.set_transfer_reviewed(false);
                ui.set_transfer_status(
                    "Отменено; данные сохранены / Cancelled; data retained".into(),
                );
            }
        });
        let weak = ui.as_weak();
        let pending = restart.clone();
        let e = estimate.clone();
        ui.on_transfer_apply(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if ui.get_dirty()
                || ui.get_data_operation_active()
                || ui.get_transfer_busy()
                || ui.get_update_running()
            {
                return;
            }
            let Some(estimate) = e.borrow().clone() else {
                return;
            };
            if estimate.target != Path::new(ui.get_transfer_target().as_str()) {
                ui.set_transfer_reviewed(false);
                ui.set_transfer_status(
                    "Путь изменён. Проверь перенос заново. / Path changed; review again.".into(),
                );
                return;
            }
            *pending.borrow_mut() = Some(estimate.target);
            ui.invoke_quit();
        });
        let weak = ui.as_weak();
        let j = join.clone();
        let token = cancel.clone();
        let e = estimate;
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated,std::time::Duration::from_millis(200),move||{
            let Some(ui)=weak.upgrade() else{return;};
            for result in rx.try_iter(){if let Some(h)=j.borrow_mut().take(){let _=h.join();}ui.set_transfer_busy(false);
                if token.load(Ordering::Acquire){continue;}
                match result {
                    Ok(estimate)=>{
                        ui.set_transfer_reviewed(estimate.target==Path::new(ui.get_transfer_target().as_str()));
                        let contents=if ui.get_english(){format!("{} files; {:.1} MB. Free space required: {:.1} MB.\nOwned models and runtimes included; external paths unchanged. Source retained. Applying closes reminders and opens a cancellable transfer window.",estimate.files,estimate.bytes as f64/1e6,estimate.peak_bytes as f64/1e6)}else{format!("{} файлов; {:.1} МБ. Нужно свободно: {:.1} МБ.\nСвои модели и среды перенесутся, внешние пути останутся. Исходник сохранится. При применении Кавычка закроется и откроет окно копирования с отменой.",estimate.files,estimate.bytes as f64/1e6,estimate.peak_bytes as f64/1e6)};
                        ui.set_transfer_status(format!("{} → {}\n{contents}\n\n{}",crate::display_path::display(&estimate.source),crate::display_path::display(&estimate.target),breakdown(&estimate,ui.get_english())).into());
                        *e.borrow_mut()=Some(estimate);
                    }
                    Err(error)=>{ui.set_transfer_reviewed(false);ui.set_transfer_status(error.into());}
                }
            }
        });
        Self {
            root,
            restart,
            cancel,
            join,
            _timer: timer,
        }
    }
    pub fn finish(self) -> Result<()> {
        self.cancel.store(true, Ordering::Release);
        if let Some(h) = self.join.borrow_mut().take() {
            let _ = h.join();
        }
        if let Some(target) = self.restart.borrow_mut().take() {
            let root = self.root.as_ref().ok_or("Installed root missing")?;
            let local =
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA missing")?);
            let mut cmd = std::process::Command::new(std::env::current_exe()?);
            cmd.arg("--move-storage").arg(root).arg(local).arg(target);
            kavychka_maintenance::windows::hide(&mut cmd);
            cmd.spawn()?;
        }
        Ok(())
    }
}
fn breakdown(estimate: &relocate::Estimate, english: bool) -> String {
    [
        ("models", "Модели", "Models"),
        ("runtimes", "Среды", "Runtimes"),
        ("audio", "Записи", "Recordings"),
        ("presets", "Пресеты", "Presets"),
        ("cache", "Кеш", "Cache"),
        ("downloads", "Загрузки", "Downloads"),
        ("recovery", "Резервы", "Recovery"),
        ("quarantine", "Карантин", "Quarantine"),
        ("exports", "Архивы", "Archives"),
        ("logs", "Журналы", "Logs"),
        ("data", "База и прочее", "Database and other files"),
    ]
    .into_iter()
    .map(|(key, ru, en)| {
        format!(
            "{}: {:.1} {}",
            if english { en } else { ru },
            estimate.breakdown.get(key).copied().unwrap_or(0) as f64 / 1e6,
            if english { "MB" } else { "МБ" }
        )
    })
    .collect::<Vec<_>>()
    .join(" · ")
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(h) = self.join.borrow_mut().take() {
            let _ = h.join();
        }
    }
}

pub fn run(root: PathBuf, local: PathBuf, target: PathBuf) -> Result<()> {
    copy_window(root, local, target, None)
}
fn copy_window(root: PathBuf, local: PathBuf, target: PathBuf, qa: Option<PathBuf>) -> Result<()> {
    crate::shell::select_backend(false).map_err(|e| e.to_string())?;
    let ui = TransferWindow::new()?;
    ui.set_dark(kavychka_platform::dark_preference().unwrap_or(false));
    ui.set_destination(crate::display_path::display(&target).into());
    ui.set_status(
        "Копирую и проверяю. Исходная папка сохраняется. / Copying and verifying; source retained."
            .into(),
    );
    let token = Arc::new(AtomicBool::new(false));
    let cancel = token.clone();
    let weak = ui.as_weak();
    ui.on_cancel(move||{cancel.store(true,Ordering::Release);if let Some(ui)=weak.upgrade(){ui.set_status("Отменяю; восстанавливаю прежние указатели… / Cancelling; restoring previous pointers…".into());}});
    let done = Rc::new(std::cell::Cell::new(false));
    let closing = done.clone();
    let cancel = token.clone();
    ui.window().on_close_requested(move || {
        if closing.get() {
            let _ = slint::quit_event_loop();
        } else {
            cancel.store(true, Ordering::Release);
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    ui.on_resume(|| {
        let _ = slint::quit_event_loop();
    });
    enum Event {
        Progress(String),
        Done(std::result::Result<(), String>),
    }
    let (tx, rx) = mpsc::channel();
    let owned_root = root.clone();
    let worker = std::thread::spawn(move || {
        let mut last = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            relocate::apply(&owned_root, &local, &target, &token, |name| {
                if last.elapsed() >= std::time::Duration::from_millis(100) {
                    last = std::time::Instant::now();
                    let _ = tx.send(Event::Progress(name.into()));
                }
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|_| {
            Err("Transfer worker failed; start the installed launcher to recover".into())
        });
        let _ = tx.send(Event::Done(result));
    });
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let qa_frames = qa.clone();
    let mut frames = 0;
    let mut progress_frame = false;
    timer.start(slint::TimerMode::Repeated,std::time::Duration::from_millis(100),move||{
        if let Some(ui)=weak.upgrade(){for event in rx.try_iter(){match event{
            Event::Progress(file)=>ui.set_status(format!("Копирую и проверяю / Copying and verifying\n{file}").into()),
            Event::Done(result)=>{done.set(true);ui.set_done(true);ui.set_status(match result{Ok(())=>"Перенос завершён и проверен. Можно открыть Кавычку. Исходная папка сохранена. / Transfer verified. Open Kavychka to continue. Source retained.".into(),Err(e)=>format!("Операция остановлена: {e}\nДанные сохранены. Запуск восстановит согласованный указатель. / Operation stopped; data retained. The launcher will recover a consistent pointer.").into()});}
        }}
        if let Some(path)=&qa_frames {
            if !done.get()&&!progress_frame {frame(&ui,&path.join("transfer-progress.png")).expect("owned QA frame");progress_frame=true;}
            if done.get(){frames+=1;match frames{
                1=>ui.set_dark(false),
                2=>{frame(&ui,&path.join("transfer-done-light.png")).expect("owned QA frame");ui.set_dark(true);},
                3=>{frame(&ui,&path.join("transfer-done-dark.png")).expect("owned QA frame");ui.invoke_resume();},
                _=>{}
            }}
        }
        }
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    let _ = worker.join();
    if qa.is_some() {
        return Ok(());
    }
    let mut command = std::process::Command::new(root.join("Kavychka.exe"));
    kavychka_maintenance::windows::hide(&mut command);
    command.spawn()?;
    Ok(())
}
fn frame(ui: &TransferWindow, path: &Path) -> Result<()> {
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
pub fn preview_test(root: PathBuf, target: PathBuf, evidence: PathBuf) -> Result<()> {
    if std::fs::read(evidence.join("p5-qa.owner"))? != b"Kavychka.P5.synthetic.v1\n"
        || root != evidence.join("program")
        || target != evidence.join("Перенесённая папка")
    {
        return Err("Owned synthetic preview fixture required".into());
    }
    let current = install::current(&root)?.ok_or("QA current absent")?;
    let store = kavychka_storage::sandbox::Sandbox::open(&current.data)?;
    let prefs = store.load()?;
    drop(store);
    crate::shell::select_backend(false).map_err(|e| e.to_string())?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(crate::display_path::display(&current.data).into());
    crate::apply(&ui, &prefs);
    ui.set_ready(true);
    ui.set_page(6);
    ui.set_profile_operation_index(2);
    ui.set_transfer_target(crate::display_path::display(&target).into());
    let controller = Controller::for_root(&ui, Some(root));
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let outcome = Rc::new(RefCell::new(None::<std::result::Result<(), String>>));
    let result = outcome.clone();
    let started = std::time::Instant::now();
    let mut phase = 0;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let check = (|| -> Result<()> {
                if started.elapsed() > std::time::Duration::from_secs(20) {
                    return Err(
                        format!("Move preview timeout: {}", ui.get_transfer_status()).into(),
                    );
                }
                match phase {
                    0 => {
                        ui.set_scroll_y(-ui.get_scroll_overflow());
                        phase = 5;
                    }
                    5 => {
                        crate::p5_profiles::click(&ui, 400., 435.);
                        phase = 1;
                    }
                    1 if ui.get_transfer_reviewed() && !ui.get_transfer_busy() => {
                        if !ui.get_transfer_status().contains("Модели: 32.0")
                            || !ui.get_transfer_status().contains("Нужно свободно")
                        {
                            return Err("Move breakdown missing/wrong".into());
                        }
                        ui.set_scroll_y(-ui.get_scroll_overflow());
                        phase = 2;
                    }
                    2 => {
                        let p = ui.window().take_snapshot()?;
                        image::save_buffer(
                            evidence.join("move-review.png"),
                            p.as_bytes(),
                            p.width(),
                            p.height(),
                            image::ColorType::Rgba8,
                        )?;
                        crate::p5_profiles::click(&ui, 400., 390.);
                        use slint::platform::WindowEvent;
                        for _ in 0..3 {
                            ui.window().dispatch_event(WindowEvent::KeyPressed {
                                text: slint::platform::Key::Tab.into(),
                            });
                            ui.window().dispatch_event(WindowEvent::KeyReleased {
                                text: slint::platform::Key::Tab.into(),
                            });
                        }
                        ui.window()
                            .dispatch_event(WindowEvent::KeyPressed { text: "\n".into() });
                        ui.window()
                            .dispatch_event(WindowEvent::KeyReleased { text: "\n".into() });
                        phase = 3;
                    }
                    3 => {
                        if ui.get_transfer_reviewed()
                            || target.exists()
                            || ui.get_transfer_busy()
                            || !ui.get_transfer_status().starts_with("Отменено")
                        {
                            return Err(
                                "Preview/cancel changed target or kept stale approval".into()
                            );
                        }
                        phase = 4;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if check.is_err() || phase == 4 {
                *result.borrow_mut() = Some(check.map_err(|e| e.to_string()));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    drop(controller);
    outcome
        .borrow_mut()
        .take()
        .ok_or("Move preview did not finish")?
        .map_err(Into::into)
}
pub fn run_test(evidence: PathBuf) -> Result<()> {
    use kavychka_storage::sandbox::Sandbox;
    if !evidence.is_absolute() || evidence.exists() {
        return Err("New absolute transfer QA folder required".into());
    }
    std::fs::create_dir_all(&evidence)?;
    std::fs::write(evidence.join("p5-qa.owner"), b"Kavychka.P5.synthetic.v1\n")?;
    let root = evidence.join("program");
    let local = evidence.join("local");
    let common = evidence.join("Общая папка");
    let target = evidence.join("Перенесённая папка");
    std::fs::create_dir(&local)?;
    drop(install::owned(&root)?);
    let current = install::Current {
        format: 1,
        version: crate::app_version().into(),
        common: common.clone(),
        data: common.join(".kavychka-next"),
        legacy: None,
    };
    {
        let mut store = Sandbox::open(&current.data)?;
        let mut prefs = store.load()?;
        prefs.user_name = "Transfer QA".into();
        prefs.desktop.setup_complete = true;
        prefs.generation.model_paths.insert(
            "text-gemma4-12b".into(),
            common.join("models/owned.gguf").to_string_lossy().into(),
        );
        store.save(&prefs)?;
    }
    std::fs::create_dir_all(common.join("models"))?;
    std::fs::write(common.join("models/owned.gguf"), vec![17u8; 32_000_000])?;
    let before = kavychka_maintenance::digest(&current.data.join("shell.sqlite3"))?;
    install::register_locator(&local, &root, &current)?;
    kavychka_maintenance::atomic(&root.join("current.json"), &current)?;
    let mut preview = std::process::Command::new(std::env::current_exe()?);
    preview
        .arg("--p5-transfer-preview-test")
        .arg(&root)
        .arg(&target)
        .arg(&evidence);
    kavychka_maintenance::windows::hide(&mut preview);
    if !preview.status()?.success() {
        return Err("Native move preview failed".into());
    }
    copy_window(
        root.clone(),
        local.clone(),
        target.clone(),
        Some(evidence.clone()),
    )?;
    let active = install::current(&root)?.ok_or("QA data pointer absent")?;
    if active.common != target
        || install::locate(&local)?
            .ok_or("QA locator absent")?
            .common_root
            != target
    {
        return Err("Transfer window did not activate target".into());
    }
    if before != kavychka_maintenance::digest(&current.data.join("shell.sqlite3"))?
        || kavychka_maintenance::digest(&common.join("models/owned.gguf"))?
            != kavychka_maintenance::digest(&target.join("models/owned.gguf"))?
    {
        return Err("Transfer source/database/model fidelity check failed".into());
    }
    let store = Sandbox::open(&current.data)?;
    if store.root != active.data
        || store.load()?.user_name != "Transfer QA"
        || Path::new(&store.load()?.generation.model_paths["text-gemma4-12b"])
            != target.join("models/owned.gguf")
    {
        return Err("Old launch path did not resolve to preserved new settings".into());
    }
    kavychka_maintenance::atomic(
        &evidence.join("result.json"),
        &serde_json::json!({"passed":true,"native_preview_callback_and_breakdown":true,"native_transfer_window":true,"offline_copy_worker":true,"private_current_and_locator_switched":true,"original_database_unchanged":true,"model_stub_sha_unchanged":true,"owned_paths_rebased":true,"old_launch_path_follows_new_data":true,"source_retained":true,"downloads":false,"global_integration_changed":false,"real_installed_package":false,"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?)?}),
    )?;
    Ok(())
}
