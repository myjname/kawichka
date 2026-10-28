//! Real installed-update controller and software-button regression.
use super::*;

pub fn run(
    output: PathBuf,
    data: PathBuf,
    expected: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() || !data.is_absolute() {
        return Err("New absolute evidence folder and absolute fixture data required".into());
    }
    fs::create_dir(&output)?;
    let root = PathBuf::from(
        std::env::var_os("KAVYCHKA_INSTALL_ROOT").ok_or("Fixture install root absent")?,
    );
    let current = kavychka_maintenance::install::current(&root)
        .map_err(|e| e.to_string())?
        .ok_or("Fixture current absent")?;
    let shown = crate::display_path::display(&data);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(shown.as_str().into());
    ui.set_app_version(crate::app_version().into());
    ui.set_theme_index(2);
    ui.set_page(7);
    let updater = crate::updater::Controller::new(&ui);
    let transfer = crate::transfer_ui::Controller::new(&ui);
    let removal = crate::removal_ui::Controller::new(&ui);
    let portable = std::env::var_os("KAVYCHKA_PORTABLE_ROOT").is_some();
    if ui.get_installed_version() != expected
        || ui.get_transfer_available() != (expected && !portable)
        || ui.get_removal_available() != (expected && !portable)
    {
        return Err("Installed update/transfer/removal capability mismatch".into());
    }
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    render(&window, &output.join("before.png"))?;
    for y in [190., 206., 222.] {
        click(&ui, 270., y);
        if ui.get_update_running() {
            break;
        }
    }
    if ui.get_update_running() != expected {
        return Err("Actual check button did not match installed capability".into());
    }
    if expected {
        let deadline = Instant::now() + Duration::from_secs(65);
        while ui.get_update_running() && Instant::now() < deadline {
            pump(&events);
            std::thread::sleep(Duration::from_millis(10));
        }
        if ui.get_update_running() || !ui.get_update_status().contains("Проверено в") {
            return Err(
                format!("Update check did not complete: {}", ui.get_update_status()).into(),
            );
        }
    }
    render(&window, &output.join("after.png"))?;
    let proof = serde_json::json!({"passed":true,"expected_installed":expected,"installed":ui.get_installed_version(),"portable":portable,
        "stored":current.data,"displayed":shown,"previous_raw_match":current.data==Path::new(&shown),
        "actual_software_pointer":true,"actual_update_check":expected,"status":ui.get_update_status().as_str(),
        "native_windows":false,"desktop_input":false,"data_changed":false});
    ui.hide()?;
    updater.finish().map_err(|e| e.to_string())?;
    transfer.finish().map_err(|e| e.to_string())?;
    drop(removal);
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&proof)?,
    )?;
    Ok(())
}
