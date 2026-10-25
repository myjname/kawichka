//! Owned-window diagnostics and explicit removal acceptance, synthetic installs.
use crate::{AppWindow, service::Backend};
use kavychka_maintenance::{Result, install, package};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
fn fixture(base: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let stage = base.join("stage");
    std::fs::create_dir(&stage)?;
    for name in [
        "app.exe",
        "Kavychka.exe",
        "next/worker.py",
        "backend/kavychka_local/engine.py",
        "assets/components.json",
        "assets/character.json",
        "assets/voice/profile.json",
    ] {
        let p = stage.join(name);
        std::fs::create_dir_all(p.parent().ok_or("Missing fixture parent")?)?;
        std::fs::write(p, b"Synthetic UI acceptance")?;
    }
    let stub = base.join("stub.exe");
    std::fs::write(&stub, b"MZ synthetic")?;
    let setup = base.join("setup.exe");
    package::append(
        &stub,
        &stage,
        package::collect(&stage, "0.1.2".into())?,
        &setup,
    )?;
    let root = base.join("program");
    let common = base.join("common");
    std::fs::create_dir(&common)?;
    kavychka_maintenance::atomic(
        &common.join("storage-owner.json"),
        &serde_json::json!({"application":"Kavychka","version":1}),
    )?;
    let current = install::apply(&root, &common, &setup, |_, c| {
        let mut s = kavychka_storage::sandbox::Sandbox::open(&c.data)?;
        let mut p = s.load()?;
        p.user_name = "PRIVATE-USER".into();
        p.generation.character_description = "PRIVATE-PROMPT".into();
        p.desktop.setup_complete = true;
        p.desktop.automatic_updates = false;
        p.generation.auto_daily = false;
        s.save(&p)?;
        Ok(())
    })?;
    let local = base.join("local");
    std::fs::create_dir(&local)?;
    install::register_locator(&local, &root, &current)?;
    Ok(root)
}
pub fn run(output: PathBuf) -> Result<()> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute evidence folder required".into());
    }
    std::fs::create_dir_all(&output)?;
    let data = output.join("diagnostic-data");
    {
        let mut s = kavychka_storage::sandbox::Sandbox::open(&data)?;
        let mut p = s.load()?;
        p.user_name = "PRIVATE-USER".into();
        p.generation.character_description = "PRIVATE-PROMPT".into();
        p.desktop.setup_complete = true;
        p.desktop.automatic_updates = false;
        p.generation.auto_daily = false;
        s.save(&p)?;
    }
    crate::shell::select_backend(true).map_err(|e| e.to_string())?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    ui.set_page(7);
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    let weak = ui.as_weak();
    let evidence = output.clone();
    let result = Rc::new(RefCell::new(None));
    let outcome = result.clone();
    let timer = slint::Timer::default();
    let started = Instant::now();
    let mut phase = 0;
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(100),move||{
        let Some(ui)=weak.upgrade()else{return};
        let check=(||->std::result::Result<(),String>{
            if started.elapsed()>Duration::from_secs(30){return Err(format!("P7 UI timeout phase {phase}: {}",ui.get_profile_status_en()))}
            if ui.get_failed(){return Err(ui.get_status().to_string())}
            match phase {
                0 if ui.get_ready()=>{ui.invoke_profile_action(6);phase=1;}
                1 if ui.get_profile_review()=="diagnostic"=>{
                    let summary=ui.get_profile_summary_en().to_string();if summary.contains("PRIVATE-"){return Err("Private field in diagnostic preview".into())}
                    let frame=ui.window().take_snapshot().map_err(|e|e.to_string())?;image::save_buffer(evidence.join("diagnostic-preview.png"),frame.as_bytes(),frame.width(),frame.height(),image::ColorType::Rgba8).map_err(|e|e.to_string())?;
                    ui.invoke_profile_action(7);phase=2;
                }
                2 if !ui.get_profile_output().is_empty()=>{
                    let b=std::fs::read(ui.get_profile_output().as_str()).map_err(|e|e.to_string())?;
                    let text=std::str::from_utf8(&b).map_err(|e|e.to_string())?;
                    if text.contains("PRIVATE-")||text.contains(&evidence.display().to_string()){return Err("Private field or absolute path in diagnostic export".into())}
                    let _:kavychka_maintenance::diagnostics::Report=serde_json::from_slice(&b).map_err(|e|e.to_string())?;
                    *outcome.borrow_mut()=Some(Ok(()));let _=slint::quit_event_loop();
                }
                _=>{}
            }
            Ok(())
        })();if let Err(error)=check{*outcome.borrow_mut()=Some(Err(error));let _=slint::quit_event_loop();}
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    timer.stop();
    drop(timer);
    ui.hide()?;
    drop(backend);
    drop(ui);
    result
        .borrow_mut()
        .take()
        .ok_or("P7 UI ended early")?
        .map_err(|e: String| e)?;
    for (name, full) in [("normal", false), ("full", true)] {
        let base = output.join(name);
        let root = fixture(&base)?;
        crate::removal_ui::run_test(
            root.clone(),
            base.join("local"),
            full,
            output.join(format!("{name}-removal.png")),
        )?;
        if root.join("versions").exists() {
            return Err("Program files retained after successful removal".into());
        }
        if base.join("common/.kavychka-next/shell.sqlite3").exists() == full {
            return Err("Removal did not follow selected data policy".into());
        }
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"diagnostic_preview_and_export_callbacks":true,"no_private_fields":true,"normal_and_full_explicit_removal":true,"synthetic_installations":true,"real_system_locator_changed":false,"physical_input_tested":false}),
        )?,
    )?;
    Ok(())
}
