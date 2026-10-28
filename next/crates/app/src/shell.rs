//! Window lifetime and presentation only. Domain decisions and disk writes live
//! in the service, never inside Slint pointer/menu callbacks.
use super::{
    AppTray, AppWindow, NoticeWindow, PetWindow,
    service::{Command, RuntimeView},
};
use kavychka_core::{
    preferences::{Language, Theme},
    reminders::Category,
};
use kavychka_platform::desktop::{self, Overlay};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{
    ComponentHandle,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc::SyncSender,
    time::Duration,
};
#[derive(Clone, Copy)]
struct DragOrigin {
    cursor: (i32, i32),
    window: (i32, i32),
}
/// Present runtime text with the same rules in the live shell and software review.
/// This helper does not create native overlays, emit audio or change preferences.
pub(crate) fn apply_runtime_status(ui: &AppWindow, view: &RuntimeView) {
    ui.set_background_running(true);
    ui.set_on_battery(view.on_battery);
    ui.set_runtime_status(view.status.as_str().into());
    ui.set_next_reminder(view.next.as_str().into());
    ui.set_pause_caption(view.countdown.as_str().into());
    ui.set_breakfast_done(view.breakfast_done);
    ui.set_pending_text(view.notice.as_ref().map_or("", |n| n.text.as_str()).into());
    ui.set_pending_key(view.notice.as_ref().map_or("", |n| n.key.as_str()).into());
}

type AlphaKey = (usize, u32, u32, u32, bool, bool);
type ImageKey = (usize, u32, u32, u32, bool);
type ClipCache = Rc<Cell<Option<(isize, u32, u32, u32, bool)>>>;
pub struct Shell {
    pub pet: PetWindow,
    pub message: NoticeWindow,
    tray: RefCell<Option<AppTray>>,
    ui: slint::Weak<AppWindow>,
    sender: SyncSender<Command>,
    pub view: RefCell<Option<RuntimeView>>,
    drag: RefCell<Option<DragOrigin>>,
    moved: Cell<bool>,
    placed: Cell<isize>,
    timer: slint::Timer,
    blink: slint::Timer,
    ticks: Cell<u64>,
    visual_key: RefCell<String>,
    visual_frames: std::sync::Arc<std::sync::Mutex<Option<crate::visuals::Frames>>>,
    visual_timer: slint::Timer,
    visual_serial: std::sync::Arc<std::sync::atomic::AtomicU64>,
    visual_started: std::time::Instant,
    visual_last: Cell<Option<ImageKey>>,
    happy_until: Cell<Option<std::time::Instant>>,
    alpha_key: Cell<Option<AlphaKey>>,
    pet_clip: ClipCache,
    message_clip: ClipCache,
    tray_failures: Cell<u32>,
    tray_retry_after: Cell<Option<std::time::Instant>>,
    pub activation: Cell<bool>,
}
pub fn handle(window: &slint::Window) -> Option<isize> {
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}
fn apply_overlay(window: &slint::Window, kind: Overlay, cache: &ClipCache) -> Result<bool, String> {
    let Some(hwnd) = handle(window) else {
        return Ok(false);
    };
    let size = window.size();
    let custom = matches!(kind, Overlay::PetAlpha);
    let key = (
        hwnd,
        size.width,
        size.height,
        window.scale_factor().to_bits(),
        custom,
    );
    if cache.get() == Some(key) {
        return Ok(false);
    }
    desktop::overlay(hwnd, kind, window.scale_factor())?;
    if !custom {
        // GDI's ellipse cuts Slint's antialiased edge. Derive the native clip
        // from the renderer itself, once per geometry rather than per frame.
        let pixels = window.take_snapshot().map_err(|e| e.to_string())?;
        let mut spans = Vec::new();
        for (y, row) in pixels
            .as_bytes()
            .chunks_exact(pixels.width() as usize * 4)
            .enumerate()
        {
            let mut start = None;
            for x in 0..=pixels.width() as usize {
                let visible = x < pixels.width() as usize && row[x * 4 + 3] > 0;
                if visible && start.is_none() {
                    start = Some(x);
                }
                if !visible && let Some(left) = start.take() {
                    spans.push([left as i32, y as i32, x as i32, y as i32 + 1]);
                }
            }
        }
        desktop::alpha_region(hwnd, &spans)?;
    }
    cache.set(Some(key));
    Ok(true)
}
fn prepare_window(
    window: &slint::Window,
    kind: Overlay,
    cache: ClipCache,
    shell: Option<std::rc::Weak<Shell>>,
) {
    window.on_winit_window_event(move |window, event| {
        if matches!(
            event,
            winit::event::WindowEvent::RedrawRequested | winit::event::WindowEvent::Resized(_)
        ) {
            window.with_winit_window(|w| {
                if w.is_resizable() {
                    w.set_resizable(false);
                }
            });
            {
                let shell = shell.as_ref().and_then(std::rc::Weak::upgrade);
                let kind = if shell.as_ref().is_some_and(|s| s.pet.get_custom_visual()) {
                    Overlay::PetAlpha
                } else {
                    kind
                };
                if apply_overlay(window, kind, &cache).unwrap_or(false)
                    && let Some(shell) = shell
                {
                    shell.alpha_key.set(None);
                    shell.update_visual();
                }
            }
        }
        EventResult::Propagate
    });
}
fn restore_main_surface(ui: &AppWindow) {
    if !ui.window().is_visible() || ui.window().is_minimized() {
        return;
    }
    // Slint 1.18.1's software snapshot invalidates the partial-rendering cache.
    // request_redraw alone can otherwise present only changed controls after
    // Windows discards a hidden/minimized window's client surface. This runs
    // once on restore, not on every redraw or timer tick.
    if let Err(error) = ui.window().take_snapshot() {
        ui.set_runtime_status(
            format!("Не удалось восстановить окно / Window redraw failed: {error}").into(),
        );
    }
    ui.window().request_redraw();
}
fn prepare_main_window(ui: &AppWindow) {
    let lost = Cell::new(false);
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        let minimized = window.with_winit_window(|w| w.is_minimized().unwrap_or(false)).unwrap_or(false);
        if minimized
            || matches!(event, winit::event::WindowEvent::Resized(size) if size.width == 0 || size.height == 0)
            || matches!(event, winit::event::WindowEvent::Occluded(true))
        {
            lost.set(true);
        } else if matches!(event, winit::event::WindowEvent::Resized(_) | winit::event::WindowEvent::Occluded(false) | winit::event::WindowEvent::Focused(true)) && lost.replace(false) {
            let weak = weak.clone();
            // Run after the backend has processed the restored dimensions.
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() { restore_main_surface(&ui); }
            });
        }
        EventResult::Propagate
    });
}
impl Shell {
    pub fn new(
        ui: &AppWindow,
        sender: SyncSender<Command>,
    ) -> Result<Rc<Self>, slint::PlatformError> {
        prepare_main_window(ui);
        let pet = PetWindow::new()?;
        let message = NoticeWindow::new()?;
        pet.window()
            .on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
        message
            .window()
            .on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
        let shell = Rc::new(Self {
            pet,
            message,
            tray: RefCell::new(None),
            ui: ui.as_weak(),
            sender,
            view: RefCell::new(None),
            drag: RefCell::new(None),
            moved: Cell::new(false),
            placed: Cell::new(0),
            timer: slint::Timer::default(),
            blink: slint::Timer::default(),
            ticks: Cell::new(0),
            visual_key: RefCell::new(String::new()),
            visual_frames: Default::default(),
            visual_timer: slint::Timer::default(),
            visual_serial: Default::default(),
            visual_started: std::time::Instant::now(),
            visual_last: Cell::new(None),
            happy_until: Cell::new(None),
            alpha_key: Cell::new(None),
            pet_clip: Default::default(),
            message_clip: Default::default(),
            tray_failures: Cell::new(0),
            tray_retry_after: Cell::new(None),
            activation: Cell::new(true),
        });
        prepare_window(
            shell.pet.window(),
            Overlay::Pet,
            shell.pet_clip.clone(),
            Some(Rc::downgrade(&shell)),
        );
        prepare_window(
            shell.message.window(),
            Overlay::Message,
            shell.message_clip.clone(),
            None,
        );
        shell.wire();
        let weak = Rc::downgrade(&shell);
        shell.visual_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(140),
            move || {
                if let Some(s) = weak.upgrade() {
                    s.update_visual();
                }
            },
        );
        let weak = Rc::downgrade(&shell);
        ui.on_runtime_update(move |json| {
            if let Some(shell) = weak.upgrade() {
                match serde_json::from_str::<RuntimeView>(json.as_str()) {
                    Ok(view) => shell.present(view),
                    Err(e) => shell.error(&e.to_string()),
                }
            }
        });
        let weak = Rc::downgrade(&shell);
        shell.timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            move || {
                if let Some(shell) = weak.upgrade() {
                    shell.maintenance();
                }
            },
        );
        let weak = Rc::downgrade(&shell);
        ui.on_runtime_stopped(move || {
            if let Some(shell) = weak.upgrade() {
                shell.view.replace(None);
                let _ = shell.pet.hide();
                let _ = shell.message.hide();
                if let Some(tray) = shell.tray.borrow_mut().take() {
                    let _ = tray.hide();
                }
                if let Some(ui) = shell.ui.upgrade() {
                    ui.set_background_running(false);
                    ui.set_ready(false);
                    let _ = ui.show();
                }
            }
        });
        Ok(shell)
    }
    fn update_visual(&self) {
        let view = self.view.borrow();
        let Some(view) = view.as_ref() else {
            return;
        };
        let Ok(frames) = self.visual_frames.lock() else {
            return;
        };
        let Some(frames) = frames.as_ref() else {
            return;
        };
        let elapsed = self.visual_started.elapsed().as_millis() as u64;
        let happy = self
            .happy_until
            .get()
            .is_some_and(|until| until > std::time::Instant::now());
        let active = view.prefs.reminders.animation_active(
            desktop::animation_allowed(),
            view.paused,
            elapsed,
            view.audio_busy || happy,
        );
        let index = frames.index(
            elapsed,
            !active,
            view.audio_busy,
            view.notice.as_ref().is_some_and(|n| n.followup > 0),
            happy,
        );
        self.pet.set_custom_visual(true);
        let size = self.pet.window().size();
        let scale = self.pet.window().scale_factor();
        let placement = frames.scene_placement(index, size.width, size.height, scale, view.paused);
        let image_key = (index, size.width, size.height, scale.to_bits(), view.paused);
        if self.visual_last.replace(Some(image_key)) != Some(image_key) {
            let image = frames.image_at(index, placement);
            self.pet.set_custom_image(image.clone());
            if let Some(ui) = self.ui.upgrade() {
                ui.set_today_image(image);
                ui.set_today_custom(true);
                ui.set_today_pixelated(frames.pixelated);
            }
        }
        self.pet.set_custom_pixelated(frames.pixelated);
        self.pet.set_custom_left(placement.left as f32 / scale);
        self.pet.set_custom_top(placement.top as f32 / scale);
        self.pet.set_custom_width(placement.width as f32 / scale);
        self.pet.set_custom_height(placement.height as f32 / scale);
        let key = (
            index,
            size.width,
            size.height,
            scale.to_bits(),
            view.paused,
            view.pending > 0,
        );
        if self.alpha_key.get() != Some(key)
            && let Some(hwnd) = handle(self.pet.window())
        {
            let spans = frames.hit_spans(
                index,
                size.width,
                size.height,
                view.paused,
                view.pending > 0,
                scale,
            );
            match desktop::alpha_region(hwnd, &spans) {
                Ok(()) => self.alpha_key.set(Some(key)),
                Err(e) => self.error(&e),
            }
        }
    }
    pub fn tray(&self) -> std::cell::Ref<'_, AppTray> {
        std::cell::Ref::map(self.tray.borrow(), |tray| {
            tray.as_ref().expect("tray belongs to a running shell")
        })
    }
    pub fn send(&self, command: Command) {
        if let Err(error) = self.sender.try_send(command) {
            self.error(&error.to_string());
        }
    }
    fn error(&self, text: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_runtime_status(text.into());
        }
    }
    pub fn open(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.window().set_minimized(false);
            let _ = ui.show();
            restore_main_surface(&ui);
            if self.activation.get()
                && let Some(hwnd) = handle(ui.window())
            {
                desktop::activate(hwnd);
            }
        }
    }
    fn quit(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let _ = ui.show();
            ui.invoke_quit();
        }
    }
    fn wire(self: &Rc<Self>) {
        macro_rules! action {
            ($instance:expr,$method:ident,$command:expr) => {{
                let weak = Rc::downgrade(self);
                $instance.$method(move || {
                    if let Some(shell) = weak.upgrade() {
                        shell.send($command);
                    }
                });
            }};
        }
        action!(self.pet, on_review, Command::Review);
        action!(self.pet, on_stretch_now, Command::Stretch);
        action!(self.pet, on_toggle_pet, Command::TogglePet);
        let weak = Rc::downgrade(self);
        self.pet.on_open_settings(move || {
            if let Some(shell) = weak.upgrade() {
                shell.open();
            }
        });
        let weak = Rc::downgrade(self);
        self.pet.on_quit(move || {
            if let Some(shell) = weak.upgrade() {
                shell.quit();
            }
        });
        let weak = Rc::downgrade(self);
        self.pet.on_pause_minutes(move |n| {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::Pause(n));
            }
        });
        let weak = Rc::downgrade(self);
        self.pet.on_drag_start(move || {
            if let Some(shell) = weak.upgrade()
                && let (Some(cursor), Some(position)) = (
                    desktop::cursor(),
                    handle(shell.pet.window()).and_then(desktop::position),
                )
            {
                shell.drag.replace(Some(DragOrigin {
                    cursor,
                    window: position,
                }));
                shell.moved.set(false);
            }
        });
        let weak = Rc::downgrade(self);
        self.pet.on_drag_move(move || {
            if let Some(shell) = weak.upgrade()
                && let Some(DragOrigin {
                    cursor: start,
                    window: position,
                }) = *shell.drag.borrow()
                && let Some(cursor) = desktop::cursor()
                && let Some(hwnd) = handle(shell.pet.window())
            {
                let dx = cursor.0 - start.0;
                let dy = cursor.1 - start.1;
                if dx.abs() + dy.abs() > 3 {
                    shell.moved.set(true);
                    let _ = desktop::place(
                        hwnd,
                        Some((position.0.saturating_add(dx), position.1.saturating_add(dy))),
                    );
                    shell.anchor();
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.pet.on_drag_end(move || {
            if let Some(shell) = weak.upgrade() {
                shell.drag.replace(None);
                if shell.moved.get()
                    && let Some(position) = handle(shell.pet.window()).and_then(desktop::position)
                {
                    shell.send(Command::MovePet(position.0, position.1));
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.message.on_confirmed(move || {
            if let Some(shell) = weak.upgrade() {
                let key = shell
                    .view
                    .borrow()
                    .as_ref()
                    .and_then(|v| v.notice.as_ref())
                    .map(|n| n.key.clone());
                if let Some(key) = key {
                    shell.send(Command::Confirm(key));
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.message.on_settings(move || {
            if let Some(shell) = weak.upgrade() {
                shell.open();
                if let Some(ui) = shell.ui.upgrade() {
                    ui.set_page(0);
                }
            }
        });
        action!(self.message, on_silence, Command::Silence);
        if let Some(ui) = self.ui.upgrade() {
            action!(ui, on_stretch_now, Command::Stretch);
            action!(ui, on_mark_breakfast, Command::Breakfast);
            action!(ui, on_review_pending, Command::Review);
            action!(ui, on_silence, Command::Silence);
            let weak = Rc::downgrade(self);
            ui.on_confirm_reminder(move |key| {
                if let Some(shell) = weak.upgrade() {
                    shell.send(Command::Confirm(key.to_string()));
                }
            });
            let weak = Rc::downgrade(self);
            ui.on_pause_minutes(move |n| {
                if let Some(shell) = weak.upgrade() {
                    shell.send(Command::Pause(n));
                }
            });
        }
    }
    fn wire_tray(self: &Rc<Self>) {
        let tray = self.tray();
        let weak = Rc::downgrade(self);
        tray.on_open_settings(move || {
            if let Some(shell) = weak.upgrade() {
                shell.open();
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_quit(move || {
            if let Some(shell) = weak.upgrade() {
                shell.quit();
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_pause_minutes(move |n| {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::Pause(n));
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_stretch_now(move || {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::Stretch);
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_mark_breakfast(move || {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::Breakfast);
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_review(move || {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::Review);
            }
        });
        let weak = Rc::downgrade(self);
        tray.on_toggle_pet(move || {
            if let Some(shell) = weak.upgrade() {
                shell.send(Command::TogglePet);
            }
        });
    }
    fn anchor(&self) {
        if self.message.window().is_visible()
            && let Some(message) = handle(self.message.window())
        {
            if let Some(pet) = handle(self.pet.window()) {
                let _ = desktop::beside(message, pet);
            } else {
                let _ = desktop::place(message, None);
            }
        }
    }
    fn style(&self) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        let dark = match view.prefs.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => kavychka_platform::dark_preference().unwrap_or(false),
        };
        self.pet.set_dark(dark);
        self.message.set_dark(dark);
        if let Some(ui) = self.ui.upgrade()
            && let Some(dark) = kavychka_platform::dark_preference()
        {
            ui.set_system_dark(dark);
        }
    }
    fn try_tray(self: &Rc<Self>, replace: bool) -> bool {
        if (self.tray.borrow().is_none() || replace)
            && self
                .tray_retry_after
                .get()
                .is_none_or(|next| std::time::Instant::now() >= next)
        {
            self.tray_retry_after
                .set(Some(std::time::Instant::now() + Duration::from_secs(5)));
            match AppTray::new() {
                Ok(tray) => {
                    let old = self.tray.replace(Some(tray));
                    drop(old);
                    self.wire_tray();
                    return true;
                }
                Err(error) => {
                    self.error(&error.to_string());
                }
            }
        }
        false
    }
    pub fn present(self: &Rc<Self>, view: RuntimeView) {
        // Explorer may not be ready at login. A tray failure must not prevent
        // consuming service views, showing the mascot or delivering notices.
        self.try_tray(false);
        let prior = self.view.borrow().clone();
        if prior.as_ref().is_some_and(|v| !v.breakfast_done) && view.breakfast_done {
            self.happy_until
                .set(Some(std::time::Instant::now() + Duration::from_secs(4)));
        }
        let key = format!(
            "{}:{}",
            view.prefs.desktop.visual_preset, view.prefs.desktop.visual_revision
        );
        if *self.visual_key.borrow() != key {
            *self.visual_key.borrow_mut() = key;
            self.visual_last.set(None);
            self.alpha_key.set(None);
            let serial = self
                .visual_serial
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if let Ok(mut f) = self.visual_frames.lock() {
                *f = None;
            }
            self.pet.set_custom_visual(false);
            if let Some(ui) = self.ui.upgrade() {
                ui.set_today_custom(false);
            }
            if !view.prefs.desktop.visual_preset.is_empty()
                && let Some(ui) = self.ui.upgrade()
            {
                let root = std::path::PathBuf::from(ui.get_storage_path().as_str());
                let id = view.prefs.desktop.visual_preset.clone();
                let frames = self.visual_frames.clone();
                let current = self.visual_serial.clone();
                let weak = self.ui.clone();
                std::thread::spawn(move || {
                    let result = (|| {
                        let preset = kavychka_storage::presets::read(&root, &id)
                            .map_err(|e| e.to_string())?;
                        if let Some((file, layout)) = preset.content.visual() {
                            crate::visuals::load(
                                &kavychka_storage::presets::inside(
                                    &kavychka_storage::presets::folder(&root, &id)
                                        .map_err(|e| e.to_string())?,
                                    file,
                                )
                                .map_err(|e| e.to_string())?,
                                layout,
                            )
                        } else {
                            Err("Selected preset is not a visual".into())
                        }
                    })();
                    if current.load(std::sync::atomic::Ordering::SeqCst) == serial {
                        match result {
                            Ok(f) => {
                                if let Ok(mut frames) = frames.lock() {
                                    *frames = Some(f);
                                }
                            }
                            Err(e) => {
                                let _ = weak.upgrade_in_event_loop(move |ui| {
                                    ui.set_runtime_status(e.into())
                                });
                            }
                        }
                    }
                });
            }
        }
        let pet_geometry_changed = prior.as_ref().is_none_or(|p| {
            p.prefs.desktop.pet_size != view.prefs.desktop.pet_size || p.paused != view.paused
        });
        self.pet.set_paused(view.paused);
        if pet_geometry_changed {
            self.pet.window().set_size(slint::LogicalSize::new(
                f32::from(view.prefs.desktop.pet_size),
                crate::visuals::Frames::window_height(
                    f32::from(view.prefs.desktop.pet_size),
                    view.paused,
                ),
            ));
        }
        let en = view.prefs.ui_language == Language::En;
        self.pet.set_english(en);
        self.pet.set_countdown(view.countdown.as_str().into());
        self.pet.set_pending(view.pending as i32);
        self.message.set_english(en);
        self.message.set_pending(view.pending as i32);
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_registered(true);
            let _ = tray.show();
            tray.set_english(en);
            tray.set_paused(view.paused);
            tray.set_has_pending(view.pending > 0);
            tray.set_pet_visible(view.prefs.reminders.pet_visible);
            tray.set_breakfast_done(view.breakfast_done);
        }
        if let Some(ui) = self.ui.upgrade() {
            apply_runtime_status(&ui, &view);
            if !ui.get_dirty() && !ui.get_busy() {
                super::apply(&ui, &view.prefs);
            }
        }
        if view.pet_permitted && view.prefs.reminders.pet_visible {
            let _ = self.pet.show();
            if let Some(hwnd) = handle(self.pet.window()) {
                let kind = if self.pet.get_custom_visual() {
                    Overlay::PetAlpha
                } else {
                    Overlay::Pet
                };
                if let Err(error) = apply_overlay(self.pet.window(), kind, &self.pet_clip) {
                    self.error(&error);
                }
                if self.placed.replace(hwnd) != hwnd
                    || prior
                        .as_ref()
                        .is_none_or(|v| v.pet_position != view.pet_position)
                {
                    let _ = desktop::place(hwnd, view.pet_position);
                } else if pet_geometry_changed {
                    let p = self.pet.window().position();
                    let _ = desktop::place(hwnd, Some((p.x, p.y)));
                }
            }
        } else {
            let _ = self.pet.hide();
        }
        if let Some(notice) = &view.notice {
            let category = notice.events[0].category;
            self.message
                .set_heading(super::service::category(category, en).into());
            self.message.set_message(notice.text.as_str().into());
            let rest = notice
                .events
                .iter()
                .any(|e| matches!(e.category, Category::Break | Category::Exercise));
            let meal = notice.events.iter().any(|e| {
                matches!(
                    e.category,
                    Category::Breakfast | Category::Lunch | Category::Dinner
                )
            });
            let label = match (en, rest, meal) {
                (false, true, true) => "Я поела / поел и встала / встал",
                (false, true, false) => "Я встала / встал",
                (false, _, true) => "Я поела / поел",
                (true, true, true) => "I ate and stood up",
                (true, true, false) => "I stood up",
                (true, _, true) => "I ate",
                _ => "OK",
            };
            self.message.set_confirm_label(label.into());
            let was_visible = self.message.window().is_visible();
            let _ = self.message.show();
            if self.message.window().is_visible()
                && (!was_visible
                    || prior
                        .as_ref()
                        .and_then(|v| v.notice.as_ref())
                        .is_none_or(|old| old.key != notice.key))
            {
                self.send(Command::Presented(notice.key.clone()));
            }
            if self.message.window().is_visible() {
                crate::p4_crash::hit("after_show");
            }
            if let Err(error) =
                apply_overlay(self.message.window(), Overlay::Message, &self.message_clip)
            {
                self.error(&error);
            }
            if prior
                .as_ref()
                .and_then(|v| v.notice.as_ref())
                .map(|n| &n.key)
                != Some(&notice.key)
            {
                self.pet
                    .set_attention(view.prefs.reminders.animation_active(
                        desktop::animation_allowed(),
                        view.paused,
                        0,
                        true,
                    ));
            }
        } else {
            let _ = self.message.hide();
            self.pet.set_attention(false);
        }
        self.view.replace(Some(view));
        self.update_visual();
        self.style();
        self.anchor();
    }
    fn maintenance(self: &Rc<Self>) {
        if self.view.borrow().is_none() {
            return;
        }
        self.style();
        self.anchor();
        let ticks = self.ticks.get() + 1;
        self.ticks.set(ticks);
        let animation = self.view.borrow().as_ref().is_some_and(|v| {
            v.prefs.reminders.animation_active(
                desktop::animation_allowed(),
                v.paused,
                self.visual_started.elapsed().as_millis() as u64,
                false,
            )
        });
        if !animation {
            self.pet.set_blink(false);
            self.pet.set_attention(false);
        }
        if animation && !self.pet.get_custom_visual() {
            self.pet.set_blink(true);
            let weak = self.pet.as_weak();
            self.blink.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(150),
                move || {
                    if let Some(pet) = weak.upgrade() {
                        pet.set_blink(false);
                        pet.set_attention(false);
                    }
                },
            );
        } else if ticks.is_multiple_of(2) {
            self.pet.set_attention(false);
        }
        if desktop::tray_registered() {
            self.tray_failures.set(0);
        } else {
            self.tray_failures.set(self.tray_failures.get() + 1);
            if self.tray_failures.get() >= 3 {
                // Recreate only this app's icon after its Shell registration is lost.
                if self.try_tray(true) {
                    let view = self.view.borrow().clone();
                    if let Some(view) = view {
                        self.present(view);
                    }
                }
                self.tray_failures.set(0);
            }
        }
        if !self.pet.window().is_visible()
            && !desktop::tray_registered()
            && let Some(ui) = self.ui.upgrade()
        {
            let _ = ui.show();
            ui.set_runtime_status(
                if ui.get_english() {
                    "Tray unavailable. Keep this window open to control Kavychka."
                } else {
                    "Трей недоступен. Это окно оставлено открытым для управления Кавычкой."
                }
                .into(),
            );
        }
    }
}
impl Drop for Shell {
    fn drop(&mut self) {
        self.timer.stop();
        self.blink.stop();
        self.visual_timer.stop();
        self.visual_serial
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(tray) = self.tray.borrow_mut().take() {
            let _ = tray.hide();
        }
        let _ = self.pet.hide();
        let _ = self.message.hide();
    }
}
pub fn select_backend(test: bool) -> Result<(), slint::PlatformError> {
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into());
    #[cfg(windows)]
    let selector = {
        use winit::platform::windows::WindowAttributesExtWindows;
        selector.with_winit_window_attributes_hook(move |a| {
            let overlay = a.title == "Kavychka · companion" || a.title == "Kavychka · reminder";
            if overlay {
                a.with_active(false)
                    .with_skip_taskbar(true)
                    .with_transparent(true)
                    .with_resizable(false)
                    .with_decorations(false)
            } else if a.title == "Кавычка · Rust preview" {
                a.with_active(!test).with_transparent(false)
            } else {
                a.with_active(!test)
            }
        })
    };
    #[cfg(not(windows))]
    let _ = test;
    selector.select()
}
