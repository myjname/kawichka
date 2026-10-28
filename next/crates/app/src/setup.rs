use crate::SetupWindow;
use kavychka_maintenance::{Result, install, package};
use slint::ComponentHandle;
use std::path::{Path, PathBuf};
pub fn prepare(data: &Path, common: &Path) -> Result<()> {
    prepare_from(data, common, None)
}
fn prepare_from(data: &Path, common: &Path, legacy: Option<&Path>) -> Result<()> {
    kavychka_maintenance::launch_context::ensure_paths(&[data, common])?;
    if data != common.join(".kavychka-next") {
        return Err("invalid shared generation path".into());
    }
    let mut store = kavychka_storage::sandbox::Sandbox::open(data)?;
    let source = legacy.or_else(|| {
        (common.join("settings.json").is_file() || common.join("library.sqlite3").is_file())
            .then_some(common)
    });
    let manager = crate::models_library::Manager::new(&mut store, source, Default::default())
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    let prefs = store.load()?;
    let clock = kavychka_platform::time::now_in(&prefs.desktop.time_zone)?;
    let engine = kavychka_core::engine::Engine::new(prefs, store.load_state()?, clock)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    kavychka_platform::time::day_in(
        &engine.prefs.schedule,
        clock,
        &engine.prefs.desktop.time_zone,
    )?;
    if !engine.prefs.desktop.visual_preset.is_empty() {
        let id = &engine.prefs.desktop.visual_preset;
        let p = kavychka_storage::presets::read(data, id)?;
        if let Some((file, layout)) = p.content.visual() {
            crate::visuals::validate(
                &kavychka_storage::presets::inside(
                    &kavychka_storage::presets::folder(data, id)?,
                    file,
                )?,
                layout,
            )
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        }
    }
    if store.has_preferences()? {
        store.save_runtime(&engine.prefs, &engine.state)?;
    }
    drop(manager);
    drop(store);
    println!(
        "Installation data/startup check passed; original source and model weights unchanged."
    );
    Ok(())
}
pub fn intercept() -> Result<bool> {
    let removal_args: Vec<_> = std::env::args_os().skip(1).collect();
    if removal_args
        .first()
        .is_some_and(|a| a == "--verify-update-context")
    {
        if removal_args.len() != 4 || !matches!(removal_args[3].to_str(), Some("yes" | "no")) {
            return Err(
                "expected --verify-update-context <new evidence> <fixture data> <yes|no>".into(),
            );
        }
        crate::controls_smoke::verify_update_context(
            PathBuf::from(&removal_args[1]),
            PathBuf::from(&removal_args[2]),
            removal_args[3] == "yes",
        )
        .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--verify-import-rollback")
    {
        if removal_args.len() != 2 {
            return Err("expected new evidence folder".into());
        }
        crate::presets_ui::verify_import_rollback(PathBuf::from(&removal_args[1]))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--verify-feedback")
    {
        if removal_args.len() != 2 {
            return Err("expected --verify-feedback <new evidence folder>".into());
        }
        crate::controls_smoke::verify_feedback(PathBuf::from(&removal_args[1]))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--verify-compact-controls")
    {
        if removal_args.len() != 2 {
            return Err("expected --verify-compact-controls <new absolute evidence folder>".into());
        }
        crate::controls_smoke::run(PathBuf::from(&removal_args[1])).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--verify-character-pack")
    {
        if removal_args.len() != 2 {
            return Err("expected --verify-character-pack <new absolute evidence folder>".into());
        }
        crate::presets_ui::verify_complete_character(PathBuf::from(&removal_args[1]))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--model-setup-cold-test")
    {
        if removal_args.len() != 2 {
            return Err("expected --model-setup-cold-test <new absolute evidence folder>".into());
        }
        crate::model_setup_smoke::run(PathBuf::from(&removal_args[1]))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--review-regression")
    {
        if removal_args.len() != 2 {
            return Err("expected --review-regression <new absolute evidence folder>".into());
        }
        crate::smoke::run_review(PathBuf::from(&removal_args[1])).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if let Some((seconds, live)) = removal_args.first().and_then(|flag| match flag.to_str() {
        Some("--soak-self-test") => Some((12, false)),
        Some("--soak-check") => Some((1200, false)),
        Some("--soak-candidate") => Some((86400, false)),
        Some("--soak-live-check") => Some((1200, true)),
        Some("--soak-live-candidate") => Some((86400, true)),
        _ => None,
    }) {
        if removal_args.len() != 2 {
            return Err("Expected qualification flag and new absolute evidence folder".into());
        }
        crate::soak::run(PathBuf::from(&removal_args[1]), seconds, live)
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--frame-contention-test")
    {
        if removal_args.len() != 3 {
            return Err(
                "expected --frame-contention-test <new evidence> <owned load fixture>".into(),
            );
        }
        crate::frame_measurement::contention(
            PathBuf::from(&removal_args[1]),
            PathBuf::from(&removal_args[2]),
        )
        .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--measure-frames")
    {
        if removal_args.len() != 3 {
            return Err(
                "expected --measure-frames <new evidence> <private model descriptor>".into(),
            );
        }
        crate::frame_measurement::run(
            PathBuf::from(&removal_args[1]),
            PathBuf::from(&removal_args[2]),
        )
        .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--clean-cpu-test")
    {
        if removal_args.len() != 3 {
            return Err(
                "expected --clean-cpu-test <new evidence> <private model descriptor>".into(),
            );
        }
        crate::p6_clean::run(
            PathBuf::from(&removal_args[1]),
            PathBuf::from(&removal_args[2]),
        )
        .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args.first().is_some_and(|a| a == "--p7-ui-test") {
        if removal_args.len() != 2 {
            return Err("expected --p7-ui-test <new evidence>".into());
        }
        crate::p7_ui::run(PathBuf::from(&removal_args[1]))?;
        return Ok(true);
    }
    if removal_args
        .first()
        .is_some_and(|a| a == "--measure-candidate")
    {
        if removal_args.len() != 2 {
            return Err("expected --measure-candidate <new evidence>".into());
        }
        crate::qualification::run(PathBuf::from(&removal_args[1]), 600)
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if removal_args.first().is_some_and(|a| a == "--remove-ui") {
        if removal_args.len() != 3 {
            return Err("expected --remove-ui <program root> <locator folder>".into());
        }
        crate::removal_ui::run(
            PathBuf::from(&removal_args[1]),
            PathBuf::from(&removal_args[2]),
        )?;
        return Ok(true);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|s| s == "--p5-transfer-preview-test")
    {
        if args.len() != 4 {
            return Err("expected private root/target/evidence".into());
        }
        crate::transfer_ui::preview_test(
            PathBuf::from(&args[1]),
            PathBuf::from(&args[2]),
            PathBuf::from(&args[3]),
        )?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--p5-transfer-ui-test") {
        if args.len() != 2 {
            return Err("expected --p5-transfer-ui-test <new evidence>".into());
        }
        crate::transfer_ui::run_test(PathBuf::from(&args[1]))?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--p5-profiles-test") {
        if args.len() != 3 {
            return Err(
                "expected --p5-profiles-test <new evidence> <synthetic FLAC fixture>".into(),
            );
        }
        crate::p5_profiles::run(PathBuf::from(&args[1]), PathBuf::from(&args[2]))
            .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--move-storage") {
        if args.len() != 4 {
            return Err(
                "expected --move-storage <installed root> <locator folder> <new shared folder>"
                    .into(),
            );
        }
        crate::transfer_ui::run(
            PathBuf::from(&args[1]),
            PathBuf::from(&args[2]),
            PathBuf::from(&args[3]),
        )?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--render-setup") {
        if args.len() != 2 {
            return Err("expected --render-setup <new evidence folder>".into());
        }
        render(PathBuf::from(&args[1]))?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--version") {
        println!("{}", crate::app_version());
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--migration-ui-test") {
        if args.len() != 2 {
            return Err("expected --migration-ui-test <new absolute evidence folder>".into());
        }
        crate::migration_smoke::run(PathBuf::from(&args[1])).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    if args
        .first()
        .is_some_and(|s| s == "--verify-python-bootstrap")
    {
        if args.len() != 2 {
            return Err("expected --verify-python-bootstrap <new owned data root>".into());
        }
        let data = PathBuf::from(&args[1]);
        let store = kavychka_storage::sandbox::Sandbox::open(&data)?;
        let exe = std::env::current_exe()?;
        let dir = exe.parent().ok_or("program path missing")?;
        let app = if dir.join("assets/components.json").is_file() {
            dir.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()?
        };
        let python = crate::bootstrap::ensure_python(&app, &data, |s| println!("{s}"))?;
        let mut command = std::process::Command::new(&python);
        command.args([
            "-B",
            "-c",
            "import sys,ssl,json,zipfile,sqlite3; print(sys.version); print(sqlite3.sqlite_version)",
        ]);
        kavychka_maintenance::windows::hide(&mut command);
        let output = command.output()?;
        if !output.status.success() {
            return Err("pinned Python probe failed".into());
        }
        let before = kavychka_maintenance::digest(&python)?;
        let again = crate::bootstrap::ensure_python(&app, &data, |_| {
            panic!("second bootstrap downloaded files")
        })?;
        if before != kavychka_maintenance::digest(&again)? {
            return Err("Python was replaced on repeat".into());
        }
        kavychka_maintenance::atomic(
            &data.join("bootstrap-result.json"),
            &serde_json::json!({"passed":true,"python_runs":true,"reused_without_network":true,"models_downloaded":false,"runtime_outside_system":true}),
        )?;
        drop(store);
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--bootstrap-worker") {
        crate::bootstrap::worker()?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--prepare-install") {
        if !matches!(args.len(), 4 | 6 | 8)
            || args[2] != "--common-root"
            || args.len() >= 6 && args[4] != "--expected-version"
            || args.len() == 8 && args[6] != "--legacy-source"
        {
            return Err("expected --prepare-install <data> --common-root <common>".into());
        }
        if let Some(version) = args.get(5)
            && version != crate::app_version()
        {
            return Err(
                "Installer manifest does not match the compiled application version".into(),
            );
        }
        prepare_from(
            Path::new(&args[1]),
            Path::new(&args[3]),
            args.get(7).map(Path::new),
        )?;
        return Ok(true);
    }
    if args.first().is_some_and(|s| s == "--apply-embedded") {
        if !matches!(args.len(), 3 | 5) || args.len() == 5 && args[3] != "--legacy-source" {
            return Err("expected --apply-embedded <install-root> <common-root>".into());
        }
        let current = install::apply_with_source(
            Path::new(&args[1]),
            Path::new(&args[2]),
            &std::env::current_exe()?,
            args.get(4).map(Path::new),
            install::probe,
        )?;
        println!("{}", serde_json::to_string(&current)?);
        return Ok(true);
    }
    let exe = std::env::current_exe()?;
    if !args.is_empty() {
        return Ok(false);
    }
    if package::payload(&exe)?.is_none() {
        return Ok(false);
    }
    show(exe)?;
    Ok(true)
}
fn render(evidence: PathBuf) -> Result<()> {
    if !evidence.is_absolute() || evidence.exists() {
        return Err("new absolute render folder required".into());
    }
    std::fs::create_dir_all(&evidence)?;
    crate::shell::select_backend(false).map_err(|e| e.to_string())?;
    let ui = SetupWindow::new()?;
    ui.set_version(crate::app_version().into());
    ui.set_program_path("E:\\Apps\\Кавычка".into());
    ui.set_data_path("E:\\Кавычка · общие модели и данные".into());
    ui.set_legacy_path("C:\\Users\\Example\\AppData\\Local\\Kavychka".into());
    ui.set_status(
        "Готово к установке. Изменений данных ещё нет. / Ready to install. No data changes yet."
            .into(),
    );
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    let mut phase = 0;
    let result = std::rc::Rc::new(std::cell::RefCell::new(
        None::<std::result::Result<(), String>>,
    ));
    let out = result.clone();
    timer.start(slint::TimerMode::Repeated,std::time::Duration::from_millis(400),move||{
        let Some(ui)=weak.upgrade() else{return};
        let check=(|| -> Result<()> {
            if phase==0 {ui.set_dark(false);ui.global::<crate::Colors>().set_dark(false);}
            if phase==1||phase==3 {let p=ui.window().take_snapshot()?;image::save_buffer(evidence.join(if phase==1{"setup-light.png"}else{"setup-dark.png"}),p.as_bytes(),p.width(),p.height(),image::ColorType::Rgba8)?;}
            if phase==2 {ui.set_dark(true);ui.global::<crate::Colors>().set_dark(true);}
            if phase==4 {kavychka_maintenance::atomic(&evidence.join("result.json"),&serde_json::json!({"passed":true,"rendered_both_themes":true,"installation_executed":false,"system_integration_changed":false}))?;}
            Ok(())
        })();
        if check.is_err()||phase==4{*out.borrow_mut()=Some(check.map_err(|e|e.to_string()));let _=slint::quit_event_loop();}phase+=1;
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    result
        .borrow_mut()
        .take()
        .ok_or("render ended early")?
        .map_err(Into::into)
}
fn show(exe: PathBuf) -> Result<()> {
    kavychka_maintenance::launch_context::require_unpacked()?;
    crate::shell::select_backend(false)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    let manifest = package::manifest(&exe)?;
    let portable = std::env::var_os("KAVYCHKA_PORTABLE_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            exe.parent()
                .filter(|p| kavychka_maintenance::portable::is_portable(p))
                .map(Path::to_owned)
        });
    if portable
        .as_ref()
        .is_some_and(|r| !kavychka_maintenance::portable::is_portable(r))
    {
        return Err("Invalid portable update root".into());
    }
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?);
    let explicit = std::env::var_os("KAVYCHKA_INSTALL_ROOT").map(PathBuf::from);
    let locator = if explicit.is_none() && portable.is_none() {
        install::locate(&local)?
    } else {
        None
    };
    let root = explicit
        .or_else(|| portable.as_ref().map(|p| p.join("program")))
        .or_else(|| locator.as_ref().map(|l| l.program_root.clone()))
        .unwrap_or_else(|| local.join("Programs/KavychkaNext"));
    let previous = install::current(&root)?;
    let common = if let Some(current) = &previous {
        current.common.clone()
    } else if let Some(portable) = &portable {
        portable.join("data")
    } else if let Some(locator) = &locator {
        locator.common_root.clone()
    } else {
        kavychka_storage::location::resolve(&local)?
    };
    if let Some(portable) = &portable
        && !kavychka_maintenance::portable::matches_layout(portable, &root, &common)
    {
        return Err("Portable program/data layout mismatch".into());
    }
    let legacy = previous
        .as_ref()
        .and_then(|c| c.legacy.clone())
        .or_else(|| locator.as_ref().and_then(|l| l.legacy.clone()))
        .or_else(|| {
            (common.join("settings.json").is_file() || common.join("library.sqlite3").is_file())
                .then_some(common.clone())
        });
    let ui = SetupWindow::new()?;
    ui.set_version(manifest.version.as_str().into());
    ui.set_program_path(crate::display_path::display(&root).into());
    ui.set_data_path(crate::display_path::display(&common).into());
    ui.set_legacy_path(
        legacy
            .map(|p| crate::display_path::display(&p))
            .unwrap_or_default()
            .into(),
    );
    ui.set_dark(kavychka_platform::dark_preference().unwrap_or(false));
    ui.set_portable(portable.is_some());
    if portable.is_some() {
        ui.set_integrations(false);
    }
    if let Some(current) = previous.as_ref()
        && let Ok(store) = kavychka_storage::sandbox::Sandbox::open(&current.data)
        && let Ok(prefs) = store.load()
    {
        ui.set_recovery_limit_mb((prefs.desktop.recovery_limit_bytes / 1_000_000) as i32);
    }
    let weak = ui.as_weak();
    let setup = exe.clone();
    let is_portable = portable.is_some();
    ui.on_install(move || {
        let Some(ui) = weak.upgrade() else { return; };
        ui.set_busy(true);
        ui.set_status("Проверка пакета и перенос данных… / Verifying package and migrating data…".into());
        let root=PathBuf::from(ui.get_program_path().as_str());
        let common=PathBuf::from(ui.get_data_path().as_str());
        let legacy=(!ui.get_legacy_path().trim().is_empty()).then(||PathBuf::from(ui.get_legacy_path().as_str()));
        let integrations=ui.get_integrations();
        let budget=ui.get_budget_edited().then_some(ui.get_recovery_limit_mb() as u64*1_000_000);
        let setup=setup.clone();
        let local=local.clone();
        let weak=ui.as_weak();
        std::thread::spawn(move || {
            let result=(|| -> Result<_> {
                if let Some(bytes)=budget { install::request_recovery_limit(&root,&common,bytes)?; }
                let current=install::apply_with_source(&root,&common,&setup,legacy.as_deref(),install::probe)?;
                let mut warnings=vec![];
                if !is_portable && let Err(e)=install::register_locator(&local,&root,&current){warnings.push(e.to_string());}
                if !is_portable && let Err(e)=kavychka_maintenance::registration::register(&root,&current.version){warnings.push(e.to_string());}
                if !is_portable && integrations && let Err(e)=kavychka_maintenance::windows::register(&root,&current){warnings.push(e.to_string());}
                let prefs=kavychka_storage::sandbox::Sandbox::open(&current.data)?.load()?;
                if !is_portable && let Err(e)=kavychka_maintenance::windows::startup(&root,prefs.desktop.run_at_login){warnings.push(e.to_string());}
                Ok(warnings)
            })();
            let _=weak.upgrade_in_event_loop(move |ui| {
                ui.set_busy(false);
                match result {
                    Ok(warnings)=>{
                        ui.set_done(true);
                        let text=if warnings.is_empty(){"Готово. Настройки, записи и пути моделей сохранены. / Ready. Settings, recordings and model paths preserved.".into()}
                        else{format!("Программа установлена. Системная интеграция требует проверки / Installed; integration needs review: {}",warnings.join("; "))};
                        ui.set_status(text.into());
                    }
                    Err(e)=>ui.set_status(e.to_string().into())
                }
            });
        });
    });
    let weak = ui.as_weak();
    ui.on_open(move || {
        if let Some(ui) = weak.upgrade() {
            let path = PathBuf::from(ui.get_program_path().as_str()).join("Kavychka.exe");
            let mut cmd = std::process::Command::new(path);
            if let Some(portable) = &portable {
                cmd.env("KAVYCHKA_PORTABLE_ROOT", portable);
            }
            kavychka_maintenance::windows::hide(&mut cmd);
            match cmd.spawn() {
                Ok(_) => {
                    let _ = slint::quit_event_loop();
                }
                Err(e) => ui.set_status(e.to_string().into()),
            }
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if weak.upgrade().is_some_and(|ui| ui.get_busy()) {
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        }
    });
    let automatic = std::env::var_os("KAVYCHKA_APPLY_UPDATE").is_some_and(|v| v == "1");
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let mut started = false;
    if automatic {
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(200),
            move || {
                if let Some(ui) = weak.upgrade() {
                    if !started {
                        started = true;
                        ui.invoke_install();
                    } else if ui.get_done() {
                        ui.invoke_open();
                    }
                }
            },
        );
    }
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}
