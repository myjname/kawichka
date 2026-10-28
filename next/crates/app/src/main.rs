//! Working P3 shell. Explicit isolated data only; no implicit live-store migration.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod bootstrap;
mod controls_smoke;
mod data_ops;
mod display_path;
mod frame_measurement;
mod frame_probe;
mod inference;
mod library_ui_smoke;
mod migration_smoke;
mod model_setup_smoke;
mod models_library;
mod models_smoke;
mod models_ui;
mod p4_audio;
mod p4_crash;
mod p4_platform;
mod p4_service;
mod p5_profiles;
mod p6_clean;
mod p7_ui;
mod platform_smoke;
mod presets_ui;
mod qualification;
mod removal_ui;
mod runtime_smoke;
mod service;
mod setup;
mod shell;
mod shell_settings;
mod shell_smoke;
mod smoke;
mod soak;
mod transfer_ui;
mod updater;
mod visuals;
use kavychka_core::preferences::{
    AnimationIntensity, AudioMode, Language, Preferences, ReminderOptions, Theme, VisualMode,
};
use kavychka_storage::sandbox::Sandbox;
use service::{Backend, Command};
use slint::ComponentHandle;
use std::{path::PathBuf, sync::mpsc::SyncSender};
slint::include_modules!();
fn app_version() -> &'static str {
    env!("KAVYCHKA_RELEASE_VERSION")
}

fn publish(weak: &slint::Weak<AppWindow>, result: Result<Preferences, String>, saved: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.set_busy(false);
        match result {
            Ok(prefs) => {
                apply(&ui, &prefs);
                ui.set_ready(true);
                ui.set_failed(false);
                ui.set_status(
                    if saved {
                        if ui.get_english() {
                            "Settings saved"
                        } else {
                            "Настройки сохранены"
                        }
                    } else {
                        ""
                    }
                    .into(),
                );
                if ui.get_exit_after_save() {
                    finish_close(&ui);
                }
            }
            Err(error) => show_error(&ui, &error),
        }
    });
}
fn show_error(ui: &AppWindow, detail: &str) {
    ui.set_exit_after_save(false);
    ui.set_failed(true);
    let prefix = if ui.get_english() {
        "Could not apply settings. Check the values or try saving again."
    } else {
        "Не удалось применить настройки. Проверь значения или повтори сохранение."
    };
    ui.set_status(format!("{prefix} {detail}").into());
}
fn format_time(minutes: u16) -> slint::SharedString {
    format!("{:02}:{:02}", minutes / 60, minutes % 60).into()
}
fn parse_time(text: &str) -> Result<u16, String> {
    let bytes = text.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || ![bytes[0], bytes[1], bytes[3], bytes[4]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return Err("HH:MM / ЧЧ:ММ".into());
    }
    let hour = (bytes[0] - b'0') as u16 * 10 + (bytes[1] - b'0') as u16;
    let minute = (bytes[3] - b'0') as u16 * 10 + (bytes[4] - b'0') as u16;
    if hour > 23 || minute > 59 {
        return Err("00:00–23:59".into());
    }
    Ok(hour * 60 + minute)
}
fn apply(ui: &AppWindow, prefs: &Preferences) {
    presets_ui::apply(ui, prefs);
    ui.set_loaded_prefs(
        serde_json::to_string(prefs)
            .expect("serializable preferences")
            .into(),
    );
    ui.set_theme_index(match prefs.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    ui.set_language_index(i32::from(prefs.ui_language == Language::En));
    ui.set_speech_index(i32::from(prefs.speech_language == Language::En));
    ui.set_user_name(prefs.user_name.as_str().into());
    ui.set_address_index(match prefs.generation.address_form.as_str() {
        "female" => 1,
        "male" => 2,
        _ => 0,
    });
    ui.set_onboarding(!prefs.desktop.setup_complete);
    if !prefs.desktop.setup_complete {
        let _ = ui.show();
    }
    shell_settings::select_saved(ui, &prefs.reminders.audio_device);
    ui.set_animation_index(match prefs.reminders.animation_intensity {
        AnimationIntensity::Off => 0,
        AnimationIntensity::Subtle => 1,
        AnimationIntensity::Expressive => 2,
    });
    let s = &prefs.schedule;
    ui.set_start_time(format_time(s.work_start));
    ui.set_end_time(format_time(s.work_end));
    ui.set_breakfast(format_time(s.breakfast));
    ui.set_lunch(format_time(s.lunch));
    ui.set_dinner(format_time(s.dinner));
    ui.set_break_minutes(s.break_every.into());
    ui.set_stretch_minutes(s.exercise_after_minutes.into());
    ui.set_meals(s.meals_enabled);
    ui.set_stretch(s.exercise_enabled);
    ui.set_monday(s.work_days.contains(&1));
    ui.set_tuesday(s.work_days.contains(&2));
    ui.set_wednesday(s.work_days.contains(&3));
    ui.set_thursday(s.work_days.contains(&4));
    ui.set_friday(s.work_days.contains(&5));
    ui.set_saturday(s.work_days.contains(&6));
    ui.set_sunday(s.work_days.contains(&0));
    ui.set_idle_minutes(prefs.idle_minutes.into());
    ui.set_allowed_clicks(prefs.allowed_clicks.into());
    ui.set_repeat_minutes(prefs.repeat_minutes.into());
    ui.set_trust_confirmation(prefs.trust_confirmation);
    let r = &prefs.reminders;
    ui.set_allow_during_dnd(r.allow_during_dnd);
    ui.set_break_repeats(r.break_repeats as i32);
    ui.set_exercise_repeats(r.exercise_repeats as i32);
    ui.set_meal_repeats(r.meal_repeats as i32);
    ui.set_audio_index(audio_index(r.audio));
    ui.set_fullscreen_audio_index(audio_index(r.fullscreen_audio));
    ui.set_fullscreen_visual_index(match r.fullscreen_visual {
        VisualMode::Both => 0,
        VisualMode::Message => 1,
        VisualMode::Nothing => 2,
    });
    ui.set_pet_visible(r.pet_visible);
    ui.set_animation(r.animation);
    models_ui::apply(ui, prefs);
    ui.set_dirty(false);
}
fn collect(ui: &AppWindow) -> Result<Preferences, String> {
    let number = |n: i32| u16::try_from(n).map_err(|_| "invalid number".to_owned());
    let language = |i| match i {
        0 => Ok(Language::Ru),
        1 => Ok(Language::En),
        _ => Err("invalid language".to_owned()),
    };
    let loaded: Preferences =
        serde_json::from_str(ui.get_loaded_prefs().as_str()).map_err(|e| e.to_string())?;
    let mut prefs = Preferences {
        format: 1,
        theme: match ui.get_theme_index() {
            0 => Theme::System,
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => return Err("invalid theme".into()),
        },
        ui_language: language(ui.get_language_index())?,
        speech_language: language(ui.get_speech_index())?,
        user_name: ui.get_user_name().to_string(),
        schedule: kavychka_core::schedule::Schedule {
            work_days: [
                (0, ui.get_sunday()),
                (1, ui.get_monday()),
                (2, ui.get_tuesday()),
                (3, ui.get_wednesday()),
                (4, ui.get_thursday()),
                (5, ui.get_friday()),
                (6, ui.get_saturday()),
            ]
            .into_iter()
            .filter_map(|(day, on)| on.then_some(day))
            .collect(),
            work_start: parse_time(ui.get_start_time().as_str())?,
            work_end: parse_time(ui.get_end_time().as_str())?,
            breakfast: parse_time(ui.get_breakfast().as_str())?,
            lunch: parse_time(ui.get_lunch().as_str())?,
            dinner: parse_time(ui.get_dinner().as_str())?,
            break_every: number(ui.get_break_minutes())?,
            meals_enabled: ui.get_meals(),
            exercise_enabled: ui.get_stretch(),
            exercise_after_minutes: number(ui.get_stretch_minutes())?,
        },
        idle_minutes: number(ui.get_idle_minutes())?,
        allowed_clicks: number(ui.get_allowed_clicks())?,
        repeat_minutes: number(ui.get_repeat_minutes())?,
        trust_confirmation: ui.get_trust_confirmation(),
        generation: loaded.generation,
        desktop: loaded.desktop,
        reminders: ReminderOptions {
            allow_during_dnd: ui.get_allow_during_dnd(),
            break_repeats: u32::try_from(ui.get_break_repeats()).map_err(|e| e.to_string())?,
            exercise_repeats: u32::try_from(ui.get_exercise_repeats())
                .map_err(|e| e.to_string())?,
            meal_repeats: u32::try_from(ui.get_meal_repeats()).map_err(|e| e.to_string())?,
            pet_visible: ui.get_pet_visible(),
            animation: ui.get_animation(),
            animation_intensity: match ui.get_animation_index() {
                0 => AnimationIntensity::Off,
                1 => AnimationIntensity::Subtle,
                2 => AnimationIntensity::Expressive,
                _ => return Err("invalid animation intensity".into()),
            },
            audio_device: ui.get_audio_device_id().to_string(),
            audio: parse_audio(ui.get_audio_index())?,
            fullscreen_audio: parse_audio(ui.get_fullscreen_audio_index())?,
            fullscreen_visual: match ui.get_fullscreen_visual_index() {
                0 => VisualMode::Both,
                1 => VisualMode::Message,
                2 => VisualMode::Nothing,
                _ => return Err("invalid fullscreen mode".into()),
            },
        },
    };
    prefs.generation.address_form = match ui.get_address_index() {
        0 => "neutral",
        1 => "female",
        2 => "male",
        _ => return Err("invalid address form".into()),
    }
    .into();
    models_ui::collect(ui, &mut prefs)?;
    presets_ui::collect(ui, &mut prefs)?;
    prefs.validate()?;
    Ok(prefs)
}
fn audio_index(mode: AudioMode) -> i32 {
    match mode {
        AudioMode::All => 0,
        AudioMode::Chime => 1,
        AudioMode::Voice => 2,
        AudioMode::Silent => 3,
    }
}
fn parse_audio(index: i32) -> Result<AudioMode, String> {
    match index {
        0 => Ok(AudioMode::All),
        1 => Ok(AudioMode::Chime),
        2 => Ok(AudioMode::Voice),
        3 => Ok(AudioMode::Silent),
        _ => Err("invalid audio mode".into()),
    }
}
fn wire(ui: &AppWindow, backend: &Backend) {
    data_ops::wire(ui, backend);
    models_ui::wire(ui, backend);
    shell_settings::wire(ui, backend);
    shell_settings::refresh(ui);
    let weak = ui.as_weak();
    ui.on_setup_next(move |direction| {
        if let Some(ui) = weak.upgrade() {
            if direction > 0
                && let Err(error) = collect(&ui)
            {
                show_error(&ui, &error);
                return;
            }
            ui.set_setup_step((ui.get_setup_step() + direction).clamp(0, 3));
            ui.set_status("".into());
            ui.set_failed(false);
            if ui.get_setup_step() == 2 {
                shell_settings::refresh(&ui);
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_finish_setup(move |models| {
        if let Some(ui) = weak.upgrade() {
            let result = (|| -> Result<(), String> {
                let mut prefs = collect(&ui)?;
                prefs.desktop.setup_complete = true;
                let base = serde_json::from_str(ui.get_loaded_prefs().as_str())
                    .map_err(|e| e.to_string())?;
                sender
                    .try_send(Command::Save(Box::new(prefs), Box::new(base)))
                    .map_err(|e| e.to_string())?;
                ui.set_busy(true);
                ui.set_page(if models { 3 } else { 0 });
                Ok(())
            })();
            if let Err(error) = result {
                show_error(&ui, &error);
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_save(move || {
        if let Some(ui) = weak.upgrade() {
            if !ui.get_ready() || ui.get_busy() {
                return;
            }
            match collect(&ui) {
                Ok(prefs) => match serde_json::from_str(ui.get_loaded_prefs().as_str()) {
                    Ok(base) => {
                        submit(&ui, &sender, Command::Save(Box::new(prefs), Box::new(base)))
                    }
                    Err(error) => show_error(&ui, &error.to_string()),
                },
                Err(error) => show_error(&ui, &error),
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_discard(move || {
        if let Some(ui) = weak.upgrade()
            && ui.get_ready()
            && !ui.get_busy()
        {
            submit(&ui, &sender, Command::Reload);
        }
    });
    let weak = ui.as_weak();
    ui.on_quit(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_close_to_background(false);
            request_exit(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_close_to_background(ui.get_background_running());
            request_exit(&ui);
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_discard_and_quit(move || {
        if let Some(ui) = weak.upgrade()
            && !ui.get_busy()
        {
            if ui.get_close_to_background() && ui.get_background_running() {
                ui.set_exit_after_save(true);
                ui.set_confirm_close(false);
                submit(&ui, &sender, Command::Reload);
            } else {
                let _ = slint::quit_event_loop();
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_save_and_quit(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_confirm_close(false);
            ui.set_exit_after_save(true);
            ui.invoke_save();
        }
    });
}
fn request_exit(ui: &AppWindow) {
    if ui.get_busy() {
        ui.set_status(
            if ui.get_english() {
                "Please wait for the save to finish."
            } else {
                "Дождись завершения сохранения."
            }
            .into(),
        );
    } else if ui.get_dirty() {
        ui.set_confirm_close(true);
    } else {
        finish_close(ui);
    }
}
fn finish_close(ui: &AppWindow) {
    ui.set_confirm_close(false);
    ui.set_exit_after_save(false);
    if ui.get_close_to_background() && ui.get_background_running() {
        let _ = ui.hide();
    } else {
        let _ = slint::quit_event_loop();
    }
}
fn submit(ui: &AppWindow, sender: &SyncSender<Command>, command: Command) {
    match sender.try_send(command) {
        Ok(()) => {
            ui.set_busy(true);
            ui.set_status("".into());
            ui.set_failed(false);
        }
        Err(error) => show_error(ui, &error.to_string()),
    }
}
struct Arguments {
    p4_service: bool,
    p4_crash: bool,
    p4_crash_child: Option<String>,
    p4_platform: bool,
    shell_acceptance: bool,
    keyboard_test: bool,
    background: bool,
    data: PathBuf,
    legacy: Option<PathBuf>,
    models_smoke: bool,
    library_ui_smoke: bool,
    smoke: Option<PathBuf>,
    native_smoke: bool,
    runtime_smoke: bool,
    runtime_recovery: bool,
    live_smoke: bool,
    platform_smoke: bool,
    failure_smoke: bool,
}
fn arguments() -> Result<Option<Arguments>, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut data = None;
    let mut legacy = None;
    let mut models_smoke = false;
    let mut library_ui_smoke = false;
    let mut smoke = None;
    let mut native_smoke = false;
    let mut runtime_smoke = false;
    let mut runtime_recovery = false;
    let mut live_smoke = false;
    let mut platform_smoke = false;
    let mut failure_smoke = false;
    let mut background = false;
    let mut p4_platform = false;
    let mut p4_service = false;
    let mut p4_crash = false;
    let mut p4_crash_child = None;
    let mut shell_acceptance = false;
    let mut keyboard_test = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--p4-service-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing P4 service evidence")?,
                ));
                p4_service = true;
            }
            Some("--p4-crash-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing P4 crash evidence")?,
                ));
                p4_crash = true;
            }
            Some("--p4-crash-child") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing P4 fault evidence")?,
                ));
                p4_crash_child = Some(
                    args.next()
                        .ok_or("missing P4 fault stage")?
                        .to_str()
                        .ok_or("invalid P4 stage")?
                        .to_owned(),
                );
            }
            Some("--p4-platform-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing P4 platform evidence")?,
                ));
                p4_platform = true;
            }
            Some("--shell-keyboard-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing keyboard evidence directory")?,
                ));
                keyboard_test = true;
            }
            Some("--shell-acceptance-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing P3 evidence directory")?,
                ));
                shell_acceptance = true;
            }
            Some("--background") => background = true,
            Some("--data-dir") if data.is_none() => {
                data = Some(PathBuf::from(args.next().ok_or("missing data directory")?))
            }
            Some("--legacy-source") if legacy.is_none() => {
                legacy = Some(PathBuf::from(args.next().ok_or("missing legacy source")?));
            }
            Some("--models-library-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing model evidence directory")?,
                ));
                models_smoke = true;
            }
            Some("--library-ui-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing library UI evidence directory")?,
                ));
                library_ui_smoke = true;
            }
            Some("--smoke-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing evidence directory")?,
                ))
            }
            Some("--native-smoke-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing evidence directory")?,
                ));
                native_smoke = true;
            }
            Some("--runtime-smoke-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing runtime evidence directory")?,
                ));
                runtime_smoke = true;
            }
            Some("--runtime-recovery-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing recovery evidence directory")?,
                ));
                runtime_smoke = true;
                runtime_recovery = true;
            }
            Some("--live-reminder-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing live evidence directory")?,
                ));
                live_smoke = true;
            }
            Some("--platform-smoke-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing platform evidence directory")?,
                ));
                platform_smoke = true;
            }
            Some("--startup-failure-test") if smoke.is_none() => {
                smoke = Some(PathBuf::from(
                    args.next().ok_or("missing failure evidence directory")?,
                ));
                failure_smoke = true;
            }
            Some("--help") => {
                println!(
                    "kavychka-app --data-dir <absolute owned preview folder> [--legacy-source <absolute original storage>]\nLegacy data is copied once into the preview; original weights/runtimes are reused in place. Never use the original storage as --data-dir. Basic reminders and FLAC playback do not need Python; local inference uses installed components. Explicit Exit stops all owned work.\nQA: --runtime-smoke-test, --models-library-test, --library-ui-test <new absolute evidence folder>."
                );
                return Ok(None);
            }
            _ => return Err("unknown/duplicate argument; use --help".into()),
        }
    }
    let data = data.ok_or("explicit --data-dir is required; use --help")?;
    if !data.is_absolute() {
        return Err("absolute --data-dir required".into());
    }
    if smoke
        .as_ref()
        .is_some_and(|p| !p.is_absolute() || p.exists())
    {
        return Err("smoke evidence directory must be absolute and new".into());
    }
    Ok(Some(Arguments {
        p4_service,
        p4_crash,
        p4_crash_child,
        p4_platform,
        shell_acceptance,
        keyboard_test,
        background,
        data,
        legacy,
        models_smoke,
        library_ui_smoke,
        smoke,
        native_smoke,
        runtime_smoke,
        runtime_recovery,
        live_smoke,
        platform_smoke,
        failure_smoke,
    }))
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    if setup::intercept().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let Some(mut args) = arguments()? else {
        return Ok(());
    };
    if let Some(output) = args.smoke {
        return if args.p4_service {
            p4_service::run(args.data, output)
        } else if args.p4_crash {
            p4_crash::run(output)
        } else if let Some(stage) = args.p4_crash_child {
            p4_crash::child(args.data, output, stage)
        } else if args.p4_platform {
            p4_platform::run(output)
        } else if args.keyboard_test {
            smoke::run_keyboard(args.data, output)
        } else if args.shell_acceptance {
            shell_smoke::run(args.data, output)
        } else if args.library_ui_smoke {
            library_ui_smoke::run(args.data, output)
        } else if args.models_smoke {
            models_smoke::run(
                args.data,
                args.legacy
                    .ok_or("--legacy-source required for model verification")?,
                output,
            )
        } else if args.failure_smoke {
            runtime_smoke::run_failure(args.data, output)
        } else if args.platform_smoke {
            platform_smoke::run(output)
        } else if args.live_smoke {
            runtime_smoke::run_live(args.data, output)
        } else if args.runtime_smoke {
            runtime_smoke::run(args.data, output, args.runtime_recovery)
        } else if args.native_smoke {
            smoke::run_native(args.data, output)
        } else {
            smoke::run(args.data, output)
        };
    }
    kavychka_maintenance::launch_context::ensure_paths(&[&args.data]).map_err(|e| e.to_string())?;
    args.data = kavychka_storage::transfer::resolve(&args.data)?;
    if let Some(root) = std::env::var_os("KAVYCHKA_INSTALL_ROOT") {
        let current = kavychka_maintenance::install::current(&PathBuf::from(root))
            .map_err(|e| e.to_string())?
            .ok_or("Installed data pointer missing")?;
        if current.data != args.data
            || !args.data.join("shell.sqlite3").is_file()
            || !std::fs::read(args.data.join("shell-sandbox.owner"))
                .is_ok_and(|b| b == b"kavychka-shell-sandbox-v1\n")
        {
            return Err("Installed data unavailable or does not match the launcher; no empty fallback was created".into());
        }
    }
    let _ui_scheduling = kavychka_platform::ui_scheduling::UiScheduling::prefer_responsive()
        .ok()
        .flatten();
    shell::select_backend(false)?;
    let ui = AppWindow::new()?;
    ui.set_app_version(app_version().into());
    ui.set_storage_path(crate::display_path::display(&args.data).into());
    ui.set_system_dark(kavychka_platform::dark_preference().unwrap_or(false));
    let theme_timer = slint::Timer::default();
    let weak = ui.as_weak();
    theme_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(2),
        move || {
            if let Some(ui) = weak.upgrade()
                && ui.get_theme_index() == 0
                && let Some(dark) = kavychka_platform::dark_preference()
            {
                ui.set_system_dark(dark);
            }
        },
    );
    let backend = if let Some(source) = args.legacy {
        Backend::start_with_legacy(args.data, source, ui.as_weak())
    } else {
        Backend::start_runtime(args.data, ui.as_weak())
    };
    wire(&ui, &backend);
    let companion = shell::Shell::new(&ui, backend.sender.clone())?;
    let updater = updater::Controller::new(&ui);
    let transfer = transfer_ui::Controller::new(&ui);
    let removal = removal_ui::Controller::new(&ui);
    if !args.background {
        ui.show()?;
    }
    slint::run_event_loop_until_quit()?;
    drop(companion);
    drop(backend);
    transfer.finish().map_err(|e| e.to_string())?;
    updater.finish().map_err(|e| e.to_string())?;
    removal.finish().map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        if std::env::args_os().len() == 1 {
            kavychka_maintenance::windows::show_error(&e.to_string());
        }
        std::process::exit(1);
    }
}
