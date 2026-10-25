//! Native QA of our own windows/tray with injected domain time, never desktop input.
use super::*;
use crate::{
    service::RuntimeView,
    shell::{Shell, handle},
};
use chrono::{DateTime, Duration as Delta, Utc};
use kavychka_core::{
    activity::Input,
    engine::{Clock, Environment},
    reminders::{Category, Event},
};
use kavychka_platform::desktop;
use slint::{
    LogicalPosition,
    platform::{PointerEventButton, WindowEvent},
    winit_030::WinitWindowAccessor,
};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
fn clock(s: i64) -> Clock {
    let utc = "2026-10-24T09:00:00Z".parse::<DateTime<Utc>>().unwrap() + Delta::seconds(s);
    Clock {
        utc,
        mono: 1_000_000 + s as u64 * 1000,
        date: utc.date_naive(),
    }
}
fn event(kind: Category, id: &str, s: i64) -> Event {
    Event {
        id: id.into(),
        category: kind,
        due: clock(s).utc,
        followup: false,
    }
}
fn step(shell: &Shell, s: i64, locked: bool, events: Option<Vec<Event>>) {
    let c = clock(s);
    shell.send(Command::ControlledStep {
        clock: c,
        environment: Environment {
            locked,
            ..Default::default()
        },
        events,
        input: if locked {
            vec![]
        } else {
            vec![(c.mono, Input::Keyboard)]
        },
    });
}
fn at(v: &RuntimeView, s: i64) -> bool {
    v.controlled_time == Some(clock(s).utc.timestamp_millis())
}
fn require(ok: bool, msg: &str) -> Result<(), Box<dyn std::error::Error>> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn click(w: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn snapshot(w: &slint::Window, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let p = w.take_snapshot()?;
    image::save_buffer(
        path,
        p.as_bytes(),
        p.width(),
        p.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
fn overlay(w: &slint::Window) -> Result<(), Box<dyn std::error::Error>> {
    let f = desktop::facts(handle(w).ok_or("missing HWND")?);
    require(
        f.no_activate
            && f.tool_window
            && f.topmost
            && f.clipped_corner
            && !f.native_frame
            && f.client_fills_window
            && !w
                .with_winit_window(|window| window.is_resizable())
                .unwrap_or(true),
        "overlay styles/region incorrect",
    )?;
    // Compare the actual Slint raster with the native clip, including faint
    // antialiased edge pixels. Interior corner samples alone miss one-pixel cuts.
    let pixels = w.take_snapshot()?;
    let width = pixels.width() as usize;
    for (y, row) in pixels.as_bytes().chunks_exact(width * 4).enumerate() {
        let visible = || {
            row.as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .filter(|(_, p)| p[3] > 0)
        };
        if let (Some((left, _)), Some((right, _))) = (visible().next(), visible().next_back()) {
            for x in [left, right] {
                require(
                    desktop::region_contains(handle(w).unwrap(), x as i32, y as i32) == Some(true),
                    &format!(
                        "native clip cuts rendered edge at ({x},{y}), size={}x{}",
                        pixels.width(),
                        pixels.height()
                    ),
                )?;
            }
        }
    }
    Ok(())
}
fn main_surface(ui: &AppWindow) -> Result<(), Box<dyn std::error::Error>> {
    let canvas = ui.global::<Colors>().get_canvas();
    let expected = [canvas.red(), canvas.green(), canvas.blue()];
    let scale = ui.window().scale_factor();
    let right = ui.window().size().width as i32 - (10. * scale) as i32;
    for (x, y) in [
        ((250. * scale) as i32, (10. * scale) as i32),
        (right, (30. * scale) as i32),
        (right, (360. * scale) as i32),
    ] {
        let actual =
            desktop::client_pixel_for_test(handle(ui.window()).ok_or("missing main HWND")?, x, y);
        require(
            actual == Some(expected),
            &format!(
                "restored client not repainted at ({x},{y}): actual={actual:?}, expected={expected:?}"
            ),
        )?;
    }
    Ok(())
}
pub fn run(
    data: PathBuf,
    output: PathBuf,
    recovery: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !recovery && data.try_exists()? && std::fs::read_dir(&data)?.next().is_some() {
        return Err("runtime QA requires empty data".into());
    }
    std::fs::create_dir_all(&output)?;
    if !recovery {
        let mut store = Sandbox::open(&data)?;
        let mut prefs = store.load()?;
        prefs.desktop.setup_complete = true;
        store.save(&prefs)?;
    }
    crate::shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_controlled(
        data.clone(),
        ui.as_weak(),
        clock(if recovery { 610 } else { 0 }),
    );
    wire(&ui, &backend);
    let shell = Shell::new(&ui, backend.sender.clone())?;
    shell.activation.set(false);
    let outcome = Rc::new(RefCell::new(None::<Result<(), String>>));
    let result = outcome.clone();
    let mut phase = 0u32;
    let sizes = [80, 116, 130, 200, 260];
    let mut size_index = 0;
    let mut reopened = 0;
    let restored = Rc::new(std::cell::Cell::new(0u32));
    let restore_count = restored.clone();
    let recovered_surface = Rc::new(std::cell::Cell::new(false));
    let recovery_verified = recovered_surface.clone();
    let mut foreground = 0isize;
    let started = Instant::now();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let companion = shell.clone();
    let evidence = output.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let pass = (|| -> Result<(), Box<dyn std::error::Error>> {
                if started.elapsed() > Duration::from_secs(75) {
                    return Err(format!("timeout at phase {phase}").into());
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string().into());
                }
                let Some(v) = companion.view.borrow().clone() else {
                    return Ok(());
                };
                if recovery {
                    match phase {
                        0 if ui.get_ready()
                            && v.notice.is_some()
                            && companion.message.window().is_visible() =>
                        {
                            require(v.handled == 3, "restart consumed events twice")?;
                            let n = v.notice.as_ref().unwrap();
                            require(
                                n.events[0].category == Category::Exercise && n.followup == 0,
                                "wrong recovery message",
                            )?;
                            overlay(companion.message.window())?;
                            snapshot(
                                companion.message.window(),
                                &evidence.join("quiet-recovery.png"),
                            )?;
                            companion.message.invoke_confirmed();
                            phase = 1;
                        }
                        1 if v.pending == 0 => {
                            companion.tray().invoke_pause_minutes(15);
                            phase = 2;
                        }
                        2 if v.paused => {
                            companion.tray().invoke_stretch_now();
                            phase = 3;
                        }
                        3 if v.notice.as_ref().is_some_and(|n| n.explicit)
                            && companion.message.window().is_visible() =>
                        {
                            snapshot(
                                companion.message.window(),
                                &evidence.join("manual-during-pause.png"),
                            )?;
                            companion.message.invoke_confirmed();
                            phase = 4;
                        }
                        4 if v.pending == 0 => {
                            *result.borrow_mut() = Some(Ok(()));
                            companion.tray().invoke_quit();
                        }
                        _ => (),
                    }
                    return Ok(());
                }
                match phase {
                    0 if ui.get_ready()
                        && companion.pet.window().is_visible()
                        && desktop::tray_registered() =>
                    {
                        overlay(companion.pet.window())?;
                        ui.set_pet_size(sizes[size_index]);
                        ui.invoke_edited();
                        ui.invoke_save();
                        phase = 40;
                    }
                    40 if !ui.get_busy()
                        && i32::from(v.prefs.desktop.pet_size) == sizes[size_index]
                        && companion.pet.window().size().width
                            == (sizes[size_index] as f32 * companion.pet.window().scale_factor())
                                .round() as u32 =>
                    {
                        overlay(companion.pet.window())?;
                        snapshot(
                            companion.pet.window(),
                            &evidence.join(format!("pet-size-{}.png", sizes[size_index])),
                        )?;
                        size_index += 1;
                        ui.set_pet_size(if size_index < sizes.len() {
                            sizes[size_index]
                        } else {
                            130
                        });
                        ui.invoke_edited();
                        ui.invoke_save();
                        if size_index == sizes.len() {
                            phase = 41;
                        }
                    }
                    41 if !ui.get_busy() && v.prefs.desktop.pet_size == 130 => {
                        ui.set_repeat_minutes(1);
                        ui.set_stretch_minutes(10);
                        ui.invoke_edited();
                        ui.invoke_save();
                        phase = 1;
                    }
                    1 if !ui.get_busy() && v.prefs.repeat_minutes == 1 => {
                        ui.set_user_name("Несохранённый черновик".into());
                        ui.invoke_edited();
                        require(
                            desktop::post_close(
                                handle(ui.window()).ok_or("missing settings HWND")?,
                            ),
                            "WM_CLOSE failed",
                        )?;
                        phase = 2;
                    }
                    2 if ui.get_confirm_close() => {
                        snapshot(ui.window(), &evidence.join("unsaved-background.png"))?;
                        ui.invoke_discard_and_quit();
                        phase = 3;
                    }
                    3 if !ui.window().is_visible() => {
                        require(
                            companion.pet.window().is_visible() && desktop::tray_registered(),
                            "background lifetime failed",
                        )?;
                        require(ui.get_user_name() == "", "discard failed")?;
                        foreground = desktop::foreground();
                        step(
                            &companion,
                            0,
                            false,
                            Some(vec![
                                event(Category::Breakfast, "qa-breakfast", 0),
                                event(Category::Break, "qa-break", 0),
                            ]),
                        );
                        phase = 4;
                    }
                    4 if v.notice.as_ref().is_some_and(|n| n.events.len() == 2)
                        && companion.message.window().is_visible() =>
                    {
                        overlay(companion.message.window())?;
                        require(
                            desktop::foreground() == foreground,
                            "notification stole focus",
                        )?;
                        snapshot(companion.pet.window(), &evidence.join("pet.png"))?;
                        snapshot(
                            companion.message.window(),
                            &evidence.join("combined-reminder.png"),
                        )?;
                        click(companion.message.window(), 200., 200.);
                        phase = 5;
                    }
                    5 if v.pending == 0 && v.breakfast_done => {
                        require(
                            !companion.message.window().is_visible(),
                            "confirm did not hide",
                        )?;
                        step(&companion, 60, false, None);
                        phase = 6;
                    }
                    6 if v.notice.as_ref().is_some_and(|n| n.followup == 1) => {
                        step(&companion, 120, false, None);
                        phase = 7;
                    }
                    7 if v.notice.as_ref().is_some_and(|n| n.followup == 2) => {
                        step(&companion, 180, false, None);
                        phase = 8;
                    }
                    8 if v.notice.as_ref().is_some_and(|n| n.followup == 3) => {
                        snapshot(
                            companion.message.window(),
                            &evidence.join("final-followup.png"),
                        )?;
                        step(&companion, 240, false, None);
                        phase = 9;
                    }
                    9 if at(&v, 240) => {
                        require(
                            v.notice.as_ref().is_some_and(|n| n.followup == 3),
                            "repeat budget exceeded",
                        )?;
                        companion.message.invoke_confirmed();
                        phase = 10;
                    }
                    10 if v.pending == 0 => {
                        step(
                            &companion,
                            400,
                            true,
                            Some(vec![event(Category::Lunch, "qa-skipped-meal", 300)]),
                        );
                        phase = 11;
                    }
                    11 if at(&v, 400) => {
                        require(
                            v.skipped > 0
                                && !companion.message.window().is_visible()
                                && !companion.pet.window().is_visible(),
                            "lock suppression failed",
                        )?;
                        step(&companion, 401, false, None);
                        phase = 12;
                    }
                    12 if at(&v, 401) => {
                        require(v.notice.is_none(), "unlock replayed backlog")?;
                        companion.tray().invoke_pause_minutes(15);
                        phase = 13;
                    }
                    13 if v.paused => {
                        snapshot(companion.pet.window(), &evidence.join("paused-pet.png"))?;
                        companion.tray().invoke_pause_minutes(0);
                        phase = 14;
                    }
                    14 if !v.paused => {
                        step(&companion, 599, false, None);
                        phase = 15;
                    }
                    15 if at(&v, 599) => {
                        require(v.notice.is_none(), "stretch fired early")?;
                        step(&companion, 600, false, None);
                        phase = 16;
                    }
                    16 if at(&v, 600)
                        && v.notice
                            .as_ref()
                            .is_some_and(|n| n.events[0].category == Category::Exercise) =>
                    {
                        require(v.handled == 3, "wrong primary count")?;
                        snapshot(
                            companion.message.window(),
                            &evidence.join("weekend-stretch.png"),
                        )?;
                        require(
                            desktop::tray_click_for_test(),
                            "native tray callback missing",
                        )?;
                        phase = 17;
                    }
                    17 if ui.window().is_visible() => {
                        main_surface(&ui)?;
                        ui.window().set_minimized(true);
                        phase = 50;
                    }
                    50 if ui.window().is_minimized() => {
                        ui.window().set_minimized(false);
                        phase = 51;
                    }
                    51 if !ui.window().is_minimized() => {
                        main_surface(&ui)?;
                        restore_count.set(restore_count.get() + 1);
                        if restore_count.get() < 20 {
                            ui.window().set_minimized(true);
                            phase = 50;
                            return Ok(());
                        }
                        let hwnd = handle(ui.window()).ok_or("missing main HWND")?;
                        require(
                            desktop::erase_client_for_test(hwnd),
                            "failed to simulate lost client surface",
                        )?;
                        let scale = ui.window().scale_factor();
                        require(
                            desktop::client_pixel_for_test(
                                hwnd,
                                (250. * scale) as i32,
                                (10. * scale) as i32,
                            ) == Some([0, 0, 0]),
                            "surface loss injection did not affect presented pixel",
                        )?;
                        ui.window().request_redraw();
                        phase = 52;
                    }
                    52 => {
                        let scale = ui.window().scale_factor();
                        require(
                            desktop::client_pixel_for_test(
                                handle(ui.window()).ok_or("missing main HWND")?,
                                (250. * scale) as i32,
                                (10. * scale) as i32,
                            ) == Some([0, 0, 0]),
                            "control redraw unexpectedly restored the simulated lost surface",
                        )?;
                        companion.open();
                        phase = 53;
                    }
                    53 => {
                        main_surface(&ui)?;
                        recovery_verified.set(true);
                        snapshot(ui.window(), &evidence.join("surface-restored.png"))?;
                        require(
                            desktop::post_close(handle(ui.window()).ok_or("missing HWND")?),
                            "close failed",
                        )?;
                        phase = 18;
                    }
                    18 if !ui.window().is_visible() => {
                        click(companion.pet.window(), 58., 58.);
                        click(companion.pet.window(), 58., 58.);
                        phase = 19;
                    }
                    19 if ui.window().is_visible() => {
                        main_surface(&ui)?;
                        reopened += 1;
                        if reopened < 50 {
                            require(
                                desktop::post_close(handle(ui.window()).ok_or("missing HWND")?),
                                "repeat close failed",
                            )?;
                            phase = 18;
                        } else {
                            require(
                                desktop::lose_tray_for_test(),
                                "could not delete own tray registration",
                            )?;
                            phase = 20;
                        }
                    }
                    20 if desktop::tray_registered() => {
                        ui.set_user_name("Сохранённый черновик".into());
                        ui.invoke_edited();
                        companion.tray().invoke_toggle_pet();
                        phase = 22;
                    }
                    22 if !v.prefs.reminders.pet_visible
                        && !companion.pet.window().is_visible() =>
                    {
                        ui.invoke_save();
                        phase = 23;
                    }
                    23 if !ui.get_busy() && v.prefs.user_name == "Сохранённый черновик" =>
                    {
                        require(
                            !v.prefs.reminders.pet_visible,
                            "stale draft overwrote tray visibility",
                        )?;
                        companion.tray().invoke_toggle_pet();
                        phase = 24;
                    }
                    24 if v.prefs.reminders.pet_visible && companion.pet.window().is_visible() => {
                        require(
                            desktop::post_close(handle(ui.window()).ok_or("missing HWND")?),
                            "final close failed",
                        )?;
                        phase = 21;
                    }
                    21 if !ui.window().is_visible() => {
                        *result.borrow_mut() = Some(Ok(()));
                        companion.tray().invoke_quit();
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(error) = pass {
                *result.borrow_mut() = Some(Err(format!("phase {phase}: {error}")));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    ui.hide()?;
    drop(timer);
    drop(shell);
    drop(backend);
    outcome
        .borrow_mut()
        .take()
        .ok_or("event loop stopped prematurely")?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    require(desktop::tray_window() == 0, "tray HWND leaked")?;
    let store = Sandbox::open(&data)?;
    let state = store.load_state()?;
    if !recovery {
        require(
            state.pending.len() == 1 && state.pending[0].events[0].category == Category::Exercise,
            "pending state missing at exit",
        )?;
    }
    drop(store);
    if !recovery {
        let child = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(&data)
            .arg("--runtime-recovery-test")
            .arg(output.join("recovery"))
            .output()?;
        if !child.status.success() {
            return Err(format!(
                "restart child failed: {}",
                String::from_utf8_lossy(&child.stderr)
            )
            .into());
        }
        std::fs::write(output.join("recovery-stdout.log"), child.stdout)?;
        let failure = std::process::Command::new(std::env::current_exe()?)
            .arg("--data-dir")
            .arg(output.join("foreign-data"))
            .arg("--startup-failure-test")
            .arg(output.join("startup-failure"))
            .output()?;
        if !failure.status.success() {
            return Err(format!(
                "startup failure QA: {}",
                String::from_utf8_lossy(&failure.stderr)
            )
            .into());
        }
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"recovery_child":recovery,"real_windows":true,"real_tray":true,"controlled_clock":true,
            "desktop_input_injected":false,"live_storage_opened":false,"os_close_message_tested":!recovery,
            "background_lifetime":true,"explicit_exit":true,"no_activate_tool_topmost_region":true,
            "double_click_reopens":!recovery,"combined_meal_break":!recovery,"three_followups":!recovery,
            "weekend_breakfast_timer":!recovery,"tray_registration_recovery":!recovery,"pending_state_persisted":true,"stale_draft_merge":!recovery,"pet_visibility_control":!recovery,
            "automatic_audio":false,"dpi_matrix_tested":false,"physical_audio_disconnect_tested":false,
            "builtin_raster_edges_preserved":true,"builtin_pet_sizes":if recovery {vec![]} else {sizes.to_vec()}
            ,"minimize_restore_cycles":restored.get(),"presented_main_surface_verified":!recovery && recovered_surface.get(),
            "lost_client_surface_simulated":recovered_surface.get(),"redraw_without_cache_reset_control_tested":recovered_surface.get()
        }))?,
    )?;
    println!("Companion/tray/reminder native integration passed (recovery={recovery}).");
    Ok(())
}
pub fn run_failure(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if data.try_exists()? {
        return Err("failure QA requires a new data directory".into());
    }
    std::fs::create_dir_all(&data)?;
    std::fs::write(data.join("foreign.txt"), b"preserve this unrelated file")?;
    std::fs::create_dir_all(&output)?;
    crate::shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    let backend = Backend::start_runtime(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    let shell = Shell::new(&ui, backend.sender.clone())?;
    shell.activation.set(false);
    let done = Rc::new(RefCell::new(false));
    let passed = done.clone();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let companion = shell.clone();
    let started = Instant::now();
    let mut failure = None;
    let errors = Rc::new(RefCell::new(None::<String>));
    let error = errors.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if ui.get_failed() {
                if ui.get_background_running()
                    || companion.pet.window().is_visible()
                    || companion.message.window().is_visible()
                    || desktop::tray_registered()
                {
                    *error.borrow_mut() =
                        Some("failed service still advertised itself as active".into());
                } else {
                    *passed.borrow_mut() = true;
                }
                ui.invoke_quit();
            } else if started.elapsed() > Duration::from_secs(5) {
                *error.borrow_mut() = Some("failed startup did not report an error".into());
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    drop(shell);
    drop(backend);
    if let Some(error) = errors.borrow_mut().take() {
        failure = Some(error);
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    require(*done.borrow(), "failure QA incomplete")?;
    require(
        std::fs::read_dir(&data)?.count() == 1
            && std::fs::read(data.join("foreign.txt"))? == b"preserve this unrelated file",
        "foreign directory was changed",
    )?;
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"foreign_data_unchanged":true,"startup_error_reported":true,"no_phantom_active_tray_or_pet":true,"explicit_exit":true}),
        )?,
    )?;
    Ok(())
}

/// Production service: real wall/monotonic clock, raw-input monitor and saved
/// schedule. Audio is deliberately silent; hardware is checked separately.
pub fn run_live(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::{Local, Timelike};
    use kavychka_core::preferences::{AudioMode, VisualMode};
    if data.try_exists()? && std::fs::read_dir(&data)?.next().is_some() {
        return Err("live QA requires empty data".into());
    }
    std::fs::create_dir_all(&output)?;
    let mut prefs = Preferences::default();
    prefs.desktop.setup_complete = true;
    let local = Local::now();
    let minute = (local.hour() * 60 + local.minute()) as u16;
    prefs.schedule.work_days = vec![0, 1, 2, 3, 4, 5, 6];
    prefs.schedule.work_start = minute;
    prefs.schedule.work_end = minute;
    prefs.schedule.meals_enabled = false;
    prefs.reminders.audio = AudioMode::Silent;
    prefs.reminders.fullscreen_visual = VisualMode::Both;
    {
        let mut store = Sandbox::open(&data)?;
        store.save(&prefs)?;
    }
    crate::shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    let backend = Backend::start_runtime(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    let shell = Shell::new(&ui, backend.sender.clone())?;
    shell.activation.set(false);
    let outcome = Rc::new(RefCell::new(None::<Result<(), String>>));
    let result = outcome.clone();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let companion = shell.clone();
    let evidence = output.clone();
    let started = Instant::now();
    let mut phase = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let pass = (|| -> Result<(), Box<dyn std::error::Error>> {
                if started.elapsed() > Duration::from_secs(12) {
                    return Err(
                        format!("live schedule timed out: {}", ui.get_runtime_status()).into(),
                    );
                }
                if ui.get_failed() {
                    return Err(ui.get_status().to_string().into());
                }
                let Some(v) = companion.view.borrow().clone() else {
                    return Ok(());
                };
                require(
                    !v.status.contains("Activity adapter unavailable"),
                    "raw-input registration failed",
                )?;
                match phase {
                    0 if v.notice.is_some() && companion.message.window().is_visible() => {
                        require(
                            v.controlled_time.is_none() && v.handled == 1,
                            "production clock/schedule path bypassed",
                        )?;
                        require(
                            v.notice.as_ref().unwrap().events[0].category == Category::Break,
                            "wrong live reminder",
                        )?;
                        overlay(companion.message.window())?;
                        snapshot(
                            companion.message.window(),
                            &evidence.join("real-clock-reminder.png"),
                        )?;
                        companion.message.invoke_confirmed();
                        phase = 1;
                    }
                    1 if v.pending == 0 => {
                        *result.borrow_mut() = Some(Ok(()));
                        companion.tray().invoke_quit();
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(error) = pass {
                *result.borrow_mut() = Some(Err(error.to_string()));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    drop(shell);
    drop(backend);
    outcome
        .borrow_mut()
        .take()
        .ok_or("live loop stopped prematurely")?
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    require(desktop::tray_window() == 0, "live tray leaked")?;
    let store = Sandbox::open(&data)?;
    let state = store.load_state()?;
    require(
        state.handled.len() == 1 && state.pending.is_empty(),
        "live confirmation not committed",
    )?;
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"production_service":true,"real_clock":true,"saved_schedule":true,"raw_input_registered":true,"real_window":true,"confirmed_and_persisted":true,"threads_joined":true,"audio_mode":"silent","live_user_store_opened":false}),
        )?,
    )?;
    println!("Production-service real-clock reminder passed.");
    Ok(())
}
