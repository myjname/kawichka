use crate::AppWindow;
use kavychka_maintenance::{Result, install, updates};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
enum Event {
    Checked(std::result::Result<Option<updates::Release>, String>),
    Downloaded(std::result::Result<PathBuf, String>),
    Prepared(std::result::Result<(PathBuf, std::fs::File), String>),
}
fn automatic_check_enabled(snapshot: &str) -> bool {
    serde_json::from_str::<kavychka_core::preferences::Preferences>(snapshot)
        .is_ok_and(|p| p.desktop.setup_complete && p.desktop.automatic_updates)
}
enum Restart {
    Update(PathBuf, std::fs::File),
    Rollback,
}
fn restart_allowed(ui: &AppWindow) -> bool {
    !ui.get_dirty()
        && !ui.get_busy()
        && !ui.get_data_operation_active()
        && !ui.get_profile_running()
        && !ui.get_transfer_busy()
}
fn prepare_restart(
    release: &updates::Release,
    path: PathBuf,
    cancel: &AtomicBool,
) -> std::result::Result<(PathBuf, std::fs::File), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Отменено / Cancelled".into());
    }
    let guard = updates::executable_guard(release, &path).map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Acquire) {
        return Err("Отменено / Cancelled".into());
    }
    Ok((path, guard))
}
struct State {
    root: Option<PathBuf>,
    ui: slint::Weak<AppWindow>,
    cancel: Arc<AtomicBool>,
    handles: RefCell<Vec<std::thread::JoinHandle<()>>>,
    running: Cell<bool>,
    release: RefCell<Option<updates::Release>>,
    download: RefCell<Option<PathBuf>>,
    restart: RefCell<Option<Restart>>,
    sender: mpsc::Sender<Event>,
}
impl State {
    fn check(&self) {
        if self.running.replace(true) {
            return;
        }
        self.cancel.store(false, Ordering::Release);
        self.download.borrow_mut().take();
        self.release.borrow_mut().take();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_update_running(true);
            ui.set_update_downloaded(false);
            ui.set_update_available(false);
            ui.set_update_version("".into());
            ui.set_update_notes("".into());
            ui.set_update_status("Проверяю GitHub Releases… / Checking GitHub Releases…".into());
        }
        let token = self.cancel.clone();
        let tx = self.sender.clone();
        self.handles.borrow_mut().push(std::thread::spawn(move || {
            let result = updates::check(crate::app_version(), &token).map_err(|e| e.to_string());
            let _ = tx.send(Event::Checked(result));
        }));
    }
    fn download(&self) {
        let Some(root) = &self.root else { return };
        let Some(release) = self.release.borrow().clone() else {
            return;
        };
        if self.running.replace(true) {
            return;
        }
        self.cancel.store(false, Ordering::Release);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_update_running(true);
            ui.set_update_status(
                format!("Загрузка / Download · {:.1} MB", release.bytes as f64 / 1e6).into(),
            );
        }
        let token = self.cancel.clone();
        let root = root.clone();
        let tx = self.sender.clone();
        self.handles.borrow_mut().push(std::thread::spawn(move || {
            let result = updates::download(&release, &root, token).map_err(|e| e.to_string());
            let _ = tx.send(Event::Downloaded(result));
        }));
    }
    fn prepare(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.root.is_none() || !restart_allowed(&ui) {
            return;
        }
        let Some(path) = self.download.borrow().clone() else {
            return;
        };
        let Some(release) = self.release.borrow().clone() else {
            return;
        };
        if self.running.replace(true) {
            return;
        }
        self.cancel.store(false, Ordering::Release);
        ui.set_update_running(true);
        ui.set_update_status(
            "Проверяю пакет перед перезапуском… / Verifying before restart…".into(),
        );
        let token = self.cancel.clone();
        let tx = self.sender.clone();
        self.handles.borrow_mut().push(std::thread::spawn(move || {
            let _ = tx.send(Event::Prepared(prepare_restart(&release, path, &token)));
        }));
    }
}
pub struct Controller {
    state: Rc<State>,
    _timer: slint::Timer,
}
impl Controller {
    pub fn new(ui: &AppWindow) -> Self {
        let root = std::env::var_os("KAVYCHKA_INSTALL_ROOT")
            .map(PathBuf::from)
            .filter(|p| {
                install::current(p).is_ok_and(|c| {
                    c.is_some_and(|c| {
                        c.data == std::path::Path::new(ui.get_storage_path().as_str())
                    })
                })
            });
        ui.set_installed_version(root.is_some());
        ui.set_rollback_available(
            root.as_ref()
                .is_some_and(|root| install::rollback_available(root)),
        );
        ui.set_update_status(if root.is_some(){"Обновления с GitHub; тексты и аудио не отправляются / GitHub updates; no texts or audio are sent"}else{"Обновления доступны в установленной версии / Updates are available in the installed version"}.into());
        let (tx, rx) = mpsc::channel();
        let state = Rc::new(State {
            root,
            ui: ui.as_weak(),
            cancel: Default::default(),
            handles: Default::default(),
            running: Cell::new(false),
            release: Default::default(),
            download: Default::default(),
            restart: Default::default(),
            sender: tx,
        });
        let s = state.clone();
        ui.on_check_updates(move || {
            if s.root.is_some() {
                s.check()
            }
        });
        let s = state.clone();
        ui.on_download_update(move || s.download());
        let s = state.clone();
        ui.on_cancel_update(move || s.cancel.store(true, Ordering::Release));
        let s = state.clone();
        ui.on_apply_update(move || s.prepare());
        let s = state.clone();
        ui.on_rollback_update(move || {
            if let Some(ui) = s.ui.upgrade()
                && restart_allowed(&ui)
                && !s.running.get()
            {
                *s.restart.borrow_mut() = Some(Restart::Rollback);
                ui.invoke_quit();
            }
        });
        let timer = slint::Timer::default();
        let s = state.clone();
        let mut last = None;
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                let Some(ui) = s.ui.upgrade() else { return };
                for event in rx.try_iter() {
                    s.running.set(false);
                    ui.set_update_running(false);
                    match event {
                        Event::Checked(result) => match result {
                            Ok(Some(release)) => {
                                ui.set_update_status(format!("Доступна версия / Version available: {}", release.version).into());
                                ui.set_update_version(release.version.as_str().into());
                                ui.set_update_notes(release.notes.as_str().into());
                                ui.set_update_available(true);
                                *s.release.borrow_mut() = Some(release);
                            }
                            Ok(None) => {
                                ui.set_update_available(false);
                                ui.set_update_status("Новых стабильных Rust-версий нет / No newer stable Rust release".into());
                            }
                            Err(error) => ui.set_update_status(error.into()),
                        },
                        Event::Downloaded(result) => match result {
                            Ok(path) => {
                                *s.download.borrow_mut() = Some(path);
                                ui.set_update_downloaded(true);
                                ui.set_update_status("Пакет и SHA-256 проверены. Можно перезапустить. / Package and SHA-256 verified. Ready to restart.".into());
                            }
                            Err(error) => ui.set_update_status(error.into()),
                        },
                        Event::Prepared(result) => match result {
                            Ok((path, guard)) => {
                                // Saving/importing may have started while hashing.
                                // Do not queue a surprise update for a later exit.
                                if restart_allowed(&ui) {
                                    *s.restart.borrow_mut() = Some(Restart::Update(path, guard));
                                    ui.invoke_quit();
                                } else {
                                    ui.set_update_status("Заверши текущую операцию и повтори перезапуск. / Finish the current operation, then request restart again.".into());
                                }
                            }
                            Err(error) => {
                                if !s.cancel.load(Ordering::Acquire) {
                                    s.download.borrow_mut().take();
                                    ui.set_update_downloaded(false);
                                }
                                ui.set_update_status(error.into());
                            }
                        },
                    }
                }
                if s.root.is_some()
                    && ui.get_ready()
                    && automatic_check_enabled(ui.get_loaded_prefs().as_str())
                    && !s.running.get()
                    && s.restart.borrow().is_none()
                    && last.is_none_or(|t: std::time::Instant| t.elapsed() > std::time::Duration::from_secs(86400))
                {
                    last = Some(std::time::Instant::now());
                    s.check();
                }
            },
        );
        Self {
            state,
            _timer: timer,
        }
    }
    pub fn finish(self) -> Result<()> {
        self.state.cancel.store(true, Ordering::Release);
        for h in self.state.handles.borrow_mut().drain(..) {
            let _ = h.join();
        }
        if let Some(root) = &self.state.root
            && let Some(restart) = self.state.restart.borrow_mut().take()
        {
            let mut execution_guard = None;
            let mut command = match restart {
                Restart::Update(path, guard) => {
                    execution_guard = Some(guard);
                    let mut cmd = std::process::Command::new(path);
                    cmd.env("KAVYCHKA_INSTALL_ROOT", root)
                        .env("KAVYCHKA_APPLY_UPDATE", "1");
                    cmd
                }
                Restart::Rollback => {
                    let mut cmd = std::process::Command::new(root.join("Kavychka.exe"));
                    cmd.arg("--rollback");
                    cmd
                }
            };
            kavychka_maintenance::windows::hide(&mut command);
            command.spawn()?;
            drop(execution_guard);
        }
        Ok(())
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.state.cancel.store(true, Ordering::Release);
        for h in self.state.handles.borrow_mut().drain(..) {
            let _ = h.join();
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn restart_preparation_refuses_corruption_and_keeps_the_execution_lock() {
        use kavychka_maintenance::{digest, package};
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        for name in [
            "app.exe",
            "Kavychka.exe",
            "next/worker.py",
            "backend/kavychka_local/engine.py",
            "assets/components.json",
            "assets/character.json",
            "assets/voice/profile.json",
        ] {
            let path = stage.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"synthetic package fixture; never executed").unwrap();
        }
        let stub = temp.path().join("stub.exe");
        std::fs::write(&stub, b"MZ synthetic fixture; never executed").unwrap();
        let path = temp.path().join("setup.exe");
        package::append(
            &stub,
            &stage,
            package::collect(&stage, "9.8.7".into()).unwrap(),
            &path,
        )
        .unwrap();
        let release = super::updates::Release {
            version: "9.8.7".into(),
            url: "https://github.com/myjname/kawichka/releases/download/next-v9.8.7/Kavychka-Next-Setup.exe".into(),
            bytes: path.metadata().unwrap().len(),
            sha256: digest(&path).unwrap(),
            notes: String::new(),
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let (_, guard) = super::prepare_restart(&release, path.clone(), &cancel).unwrap();
        #[cfg(windows)]
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(guard);
        std::fs::write(&path, vec![0u8; release.bytes as usize]).unwrap();
        assert!(super::prepare_restart(&release, path, &cancel).is_err());
    }
    #[test]
    fn cancelled_preparation_does_not_open_the_package() {
        let release = super::updates::Release {
            version: "9.8.7".into(),
            url: String::new(),
            bytes: 1234,
            sha256: "0".repeat(64),
            notes: String::new(),
        };
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let error = super::prepare_restart(
            &release,
            std::path::PathBuf::from("missing-qa-setup.exe"),
            &cancel,
        )
        .unwrap_err();
        assert!(error.contains("Cancelled"));
    }
    #[test]
    fn auto_check_waits_for_committed_setup_and_respects_disabled_updates() {
        let mut p = kavychka_core::preferences::Preferences::default();
        assert!(!super::automatic_check_enabled(
            &serde_json::to_string(&p).unwrap()
        ));
        p.desktop.setup_complete = true;
        assert!(super::automatic_check_enabled(
            &serde_json::to_string(&p).unwrap()
        ));
        p.desktop.automatic_updates = false;
        assert!(!super::automatic_check_enabled(
            &serde_json::to_string(&p).unwrap()
        ));
        assert!(!super::automatic_check_enabled("invalid"));
    }
}
