use kavychka_storage::{guard::LegacyGuard, preview};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

static CANCEL: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
unsafe extern "system" fn on_console_event(event: u32) -> i32 {
    if event == 0 || event == 1 {
        CANCEL.store(true, Ordering::Relaxed);
        1
    } else {
        0
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(command) = args.next() else {
        help();
        return Ok(());
    };
    if command == "--help" || command == "-h" {
        help();
        return Ok(());
    }
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if !["--source", "--destination", "--max-bytes"]
            .iter()
            .any(|allowed| key == *allowed)
        {
            return Err("неизвестный параметр; используйте --help".into());
        }
        let value = args.next().ok_or("после параметра требуется значение")?;
        if options.insert(key, value).is_some() {
            return Err("параметр указан дважды".into());
        }
    }
    let source = PathBuf::from(
        options
            .get(std::ffi::OsStr::new("--source"))
            .ok_or("укажите --source")?,
    );
    #[cfg(windows)]
    unsafe {
        if windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_console_event), 1)
            == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    if command == "check-source" {
        if options.len() != 1 {
            return Err("check-source принимает только --source".into());
        }
        let inventory = preview::inventory(&source, &CANCEL)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "files":inventory.files.len(),"source_bytes":inventory.copy_bytes,
                "source_fingerprint":inventory.fingerprint,"source_modified":false,
                "models_hashed_or_copied":false,"live_storage_switched":false
            }))?
        );
    } else if command == "preview-import" {
        let destination = PathBuf::from(
            options
                .get(std::ffi::OsStr::new("--destination"))
                .ok_or("укажите --destination")?,
        );
        let limit = options
            .get(std::ffi::OsStr::new("--max-bytes"))
            .map(|value| {
                value
                    .to_str()
                    .ok_or("неверный лимит")?
                    .parse::<u64>()
                    .map_err(|_| "неверный лимит")
            })
            .transpose()?
            .unwrap_or(2_000_000_000);
        let guard = LegacyGuard::acquire()?;
        let report = preview::prepare(&source, &destination, limit, &CANCEL, &guard, |phase| {
            eprintln!("{}", json!({"phase":phase}));
        })?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "phase":report.phase,"files":report.files,"rows":report.rows,
                "models_copied":report.models_copied,"live_storage_switched":report.live_storage_switched
            }))?
        );
    } else {
        return Err("неизвестная команда; используйте --help".into());
    }
    Ok(())
}
fn help() {
    println!(
        "Кавычка Next — инструменты разработки, не замена установленной версии.\n\n\
check-source --source <папка>\n  Проверить состав источника без изменений.\n\n\
preview-import --source <папка> --destination <отдельная папка> [--max-bytes <число>]\n  Создать проверенную пробную копию. Исходник не меняется, модели не копируются.\n  Действующее хранилище не переключается. Ctrl+C отменяет операцию.\n\n\
Используйте копию пользовательских данных для испытаний. Для импорта старая Кавычка\n\
должна быть закрыта; программа не завершает её принудительно."
    );
}
