//! Removal runs from a private copied executable after the app releases its DB.
use crate::{AppWindow, RemovalWindow};
use kavychka_maintenance::{Result, removal};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
};
pub struct Controller {
    pending: Rc<Cell<bool>>,
    root: Option<PathBuf>,
}
impl Controller {
    pub fn new(ui: &AppWindow) -> Self {
        let pending = Rc::new(Cell::new(false));
        let flag = pending.clone();
        let weak = ui.as_weak();
        let root = std::env::var_os("KAVYCHKA_INSTALL_ROOT")
            .map(PathBuf::from)
            .filter(|r| {
                kavychka_maintenance::install::current(r).is_ok_and(|v| {
                    v.is_some_and(|v| {
                        crate::installation::same_data_path(
                            &v.data,
                            std::path::Path::new(ui.get_storage_path().as_str()),
                        )
                    })
                })
            });
        let available = root.is_some() && std::env::var_os("KAVYCHKA_PORTABLE_ROOT").is_none();
        ui.set_removal_available(available);
        ui.on_open_removal(move || {
            if let Some(ui) = weak.upgrade()
                && available
                && !ui.get_dirty()
                && !ui.get_data_operation_active()
                && !ui.get_update_running()
                && !ui.get_transfer_busy()
            {
                flag.set(true);
                ui.invoke_quit();
            }
        });
        Self { pending, root }
    }
    pub fn finish(self) -> Result<()> {
        if !self.pending.get() {
            return Ok(());
        }
        let root = self.root.ok_or("Installed root unavailable")?;
        let local =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?);
        let scratch = std::env::temp_dir().join(format!(
            "Kavychka-remove-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::create_dir(&scratch)?;
        let exe = scratch.join("Kavychka-Remove.exe");
        std::fs::copy(std::env::current_exe()?, &exe)?;
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["--remove-ui"]).arg(root).arg(local);
        kavychka_maintenance::windows::hide(&mut cmd);
        cmd.spawn()?;
        Ok(())
    }
}
pub fn run(root: PathBuf, local: PathBuf) -> Result<()> {
    window(root, local, None)
}
pub fn run_test(root: PathBuf, local: PathBuf, full: bool, image: PathBuf) -> Result<()> {
    window(root, local, Some((full, image)))
}
fn window(root: PathBuf, local: PathBuf, qa: Option<(bool, PathBuf)>) -> Result<()> {
    if qa.is_none() {
        crate::shell::select_backend(false).map_err(|e| e.to_string())?;
    }
    let ui = RemovalWindow::new()?;
    if let Ok(c) = kavychka_maintenance::removal::current(&root)
        && c.data.join("shell.sqlite3").is_file()
        && let Ok(s) = kavychka_storage::sandbox::Sandbox::open(&c.data)
        && let Ok(p) = s.load()
    {
        ui.set_english(p.ui_language == kavychka_core::preferences::Language::En);
    }
    ui.set_dark(kavychka_platform::dark_preference().unwrap_or(false));
    if let Some((full, _)) = &qa {
        ui.set_full(*full);
    }
    let plan = Rc::new(RefCell::new(None::<removal::Plan>));
    let busy = Rc::new(Cell::new(false));
    let removed = Rc::new(Cell::new(false));
    let cancelling = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let removing = Rc::new(Cell::new(false));
    let (tx, rx) = mpsc::channel();
    enum Event {
        Preview(std::result::Result<removal::Plan, String>),
        Removed(std::result::Result<(), String>),
    }
    let weak = ui.as_weak();
    let r = root.clone();
    let p = plan.clone();
    let b = busy.clone();
    let sender = tx.clone();
    let token = cancelling.clone();
    let phase = removing.clone();
    ui.on_preview(move || {
        if b.replace(true) {
            return;
        }
        let Some(ui) = weak.upgrade() else {
            b.set(false);
            return;
        };
        ui.set_busy(true);
        phase.set(false);
        ui.set_removing(false);
        ui.set_reviewed(false);
        *p.borrow_mut() = None;
        ui.set_summary("Считаю точный состав… / Inspecting exact contents…".into());
        let root = r.clone();
        let full = ui.get_full();
        let sender = sender.clone();
        token.store(false, std::sync::atomic::Ordering::Release);
        let token = token.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                removal::preview_with_cancel(&root, full, &token).map_err(|e| e.to_string())
            })
            .unwrap_or_else(|_| Err("Removal preview worker failed".into()));
            let _ = sender.send(Event::Preview(result));
        });
    });
    let weak = ui.as_weak();
    let p = plan.clone();
    let b = busy.clone();
    let sender = tx;
    let deleting = removing.clone();
    let synthetic = qa.is_some();
    ui.on_remove(move || {
        let Some(ui) = weak.upgrade() else { return };
        if b.get() || !ui.get_confirmed() {
            return;
        }
        let Some(plan) = p.borrow().clone() else {
            return;
        };
        if plan.full != ui.get_full() {
            return;
        }
        b.set(true);
        deleting.set(true);
        ui.set_removing(true);
        ui.set_busy(true);
        ui.set_reviewed(false);
        ui.set_summary("Удаляю выбранное… / Removing selected contents…".into());
        let local = local.clone();
        let sender = sender.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                removal::apply(&plan, &local, |root| {
                    if synthetic {
                        Ok(())
                    } else {
                        kavychka_maintenance::windows::unregister(root)
                    }
                })
                .map_err(|e| e.to_string())
            })
            .unwrap_or_else(|_| {
                Err("Removal worker failed; run setup to recover program files".into())
            });
            let _ = sender.send(Event::Removed(result));
        });
    });
    let weak = ui.as_weak();
    let b = busy.clone();
    let p = plan;
    let finished = removed.clone();
    let timer = slint::Timer::default();
    let qa_outcome = Rc::new(RefCell::new(None));
    let qa_result = qa_outcome.clone();
    let qa_started = std::time::Instant::now();
    let mut qa_review_since = None;
    let token = cancelling.clone();
    timer.start(slint::TimerMode::Repeated,std::time::Duration::from_millis(100),move||{
        let Some(ui)=weak.upgrade()else{return};
        for event in rx.try_iter(){b.set(false);ui.set_busy(false);if token.load(std::sync::atomic::Ordering::Acquire){let _=slint::quit_event_loop();return;}match event{
            Event::Preview(Ok(plan))=>{
                let en=ui.get_english();
                let decision=match (en,plan.full){(true,true)=>"Program AND ALL owned shared storage will be removed, including models and recordings.",(false,true)=>"Удалятся программа И ВСЁ собственное общее хранилище, включая модели и записи.",(true,false)=>"Program removed. Settings, models, recordings and locator retained for reinstall.",(false,false)=>"Удалится программа. Настройки, модели, записи и указатель сохранятся для переустановки."};
                let storage=match (en,plan.full){(true,true)=>"Removing",(true,false)=>"Keeping",(false,true)=>"Удаляется",(false,false)=>"Сохраняется"};
                let external=plan.external_paths_retained.iter().map(|p|crate::display_path::display(p)).collect::<Vec<_>>().join("\n");
                let files=if en{"files"}else{"файлов"};let retained=if en{"External paths retained"}else{"Внешние пути сохраняются"};let warning=if en{"Full removal is irreversible. Export a profile first."}else{"Полное удаление необратимо. Экспортируй профиль заранее."};
                ui.set_summary(format!("{decision}\n{}\n{} {files}; {:.1} MB\n\n{storage}: {}\n{retained}:\n{external}\n\n{warning}",crate::display_path::display(&plan.program),plan.files.len(),plan.bytes as f64/1e6,crate::display_path::display(&plan.common)).into());
                ui.set_reviewed(plan.full==ui.get_full());*p.borrow_mut()=Some(plan);
            }
            Event::Preview(Err(error))|Event::Removed(Err(error))=>{ui.set_summary(format!("Не выполнено / Failed: {error}\nДанные не скрываются новой пустой библиотекой. При частичном удалении программы запусти установщик повторно. / If program removal was interrupted, run setup again.").into());}
            Event::Removed(Ok(()))=>{finished.set(true);ui.set_done(true);ui.set_summary("Удаление завершено. / Removal complete.".into());}
        }}
        if let Some((_,image))=&qa {
            if qa_started.elapsed()>std::time::Duration::from_secs(45){*qa_result.borrow_mut()=Some(Err("Removal UI timeout".to_string()));let _=slint::quit_event_loop();}
            else if ui.get_reviewed()&&!ui.get_busy(){
                let since=qa_review_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed()<std::time::Duration::from_millis(250){return;}
                if let Ok(frame)=ui.window().take_snapshot(){let _=image::save_buffer(image,frame.as_bytes(),frame.width(),frame.height(),image::ColorType::Rgba8);}
                ui.set_confirmed(true);ui.invoke_remove();
            }else if ui.get_done(){*qa_result.borrow_mut()=Some(Ok(()));ui.invoke_dismiss();}
        }
    });
    let b = busy.clone();
    let r = removing.clone();
    let token = cancelling.clone();
    ui.on_dismiss(move || {
        if b.get() && !r.get() {
            token.store(true, std::sync::atomic::Ordering::Release);
        } else if !b.get() {
            let _ = slint::quit_event_loop();
        }
    });
    let b = busy;
    let r = removing;
    let token = cancelling;
    ui.window().on_close_requested(move || {
        if b.get() && !r.get() {
            token.store(true, std::sync::atomic::Ordering::Release);
        } else if !b.get() {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    ui.show()?;
    ui.invoke_preview();
    slint::run_event_loop_until_quit()?;
    timer.stop();
    if let Some(result) = qa_outcome.borrow_mut().take() {
        result.map_err(|e: String| e)?;
    }
    if !removed.get()
        && std::env::var_os("KAVYCHKA_REMOVAL_RESUME").is_none_or(|s| s != "0")
        && root.join("Kavychka.exe").exists()
        && kavychka_maintenance::install::current(&root).is_ok_and(|c| c.is_some())
    {
        let mut cmd = std::process::Command::new(root.join("Kavychka.exe"));
        kavychka_maintenance::windows::hide(&mut cmd);
        cmd.spawn()?;
    }
    Ok(())
}
