//! Isolated functional + layout acceptance for the user's 0.1.34 feedback.
use super::*;
use kavychka_storage::library::Phrase;
use serde_json::json;

pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut prefs = store.load()?;
    prefs.desktop.setup_complete = true;
    prefs.desktop.automatic_updates = false;
    prefs.generation.auto_daily = false;
    prefs.generation.text_enabled = false;
    prefs.generation.tts_model = "system".into();
    store.save(&prefs)?;
    let day = kavychka_platform::time::now_in(&prefs.desktop.time_zone)?.date;
    for (id, date, approved, rejected) in [
        ("ready", day, true, false),
        ("played", day, true, false),
        ("excluded", day, false, true),
        ("old-unused", day - chrono::Days::new(1), true, false),
    ] {
        let row: Phrase = serde_json::from_value(
            json!({"id":id,"text":format!("Тест {id}: небольшая пауза поможет вернуться к задаче со свежими силами. Даже длинная фраза должна читаться без наложения на иконки действий справа."),"category":"break","model":"system","profile":"system","day":date.to_string(),"audio_path":null,"approved":approved,"deleted":false,"rejected":rejected,"used_at":null,"created_at":day.and_hms_opt(12,0,0).unwrap().and_utc().to_rfc3339(),"followup":false,"language":"ru"}),
        )?;
        store.accept_phrase(&row, None)?;
    }
    store.save_delivery(
        &prefs,
        &Default::default(),
        Some(("completed-notice", "played")),
    )?;
    store.delivery_phase("completed-notice", "audio_completed")?;
    drop(store);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_app_version(crate::app_version().into());
    ui.set_audio_refreshing(true);
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    wait(&events, "current list and daily support", || {
        ui.get_ready()
            && ui.get_library_items().row_count() == 3
            && ui.get_support_texts().row_count() == 4
    })?;
    let items = ui.get_library_items();
    for (id, state) in [("ready", "●"), ("played", "✓"), ("excluded", "×")] {
        if !items.iter().any(|r| r.identifier == id && r.state == state) {
            return Err(format!("Wrong library state for {id}").into());
        }
    }
    ui.set_library_today(false);
    ui.invoke_library_filter();
    wait(&events, "history separate from today", || {
        ui.get_library_items().row_count() == 1
            && ui.get_library_items().row_data(0).unwrap().identifier == "old-unused"
    })?;
    ui.set_library_today(true);
    ui.invoke_library_filter();
    wait(&events, "current restored", || {
        ui.get_library_items().row_count() == 3
    })?;
    ui.invoke_select_phrase("ready".into(), false);
    wait(&events, "exclude", || {
        ui.get_library_items()
            .iter()
            .any(|r| r.identifier == "ready" && r.excluded)
    })?;
    ui.invoke_select_phrase("ready".into(), true);
    wait(&events, "keep", || {
        ui.get_library_items()
            .iter()
            .any(|r| r.identifier == "ready" && r.kept && !r.excluded)
    })?;
    // Capture real UI states; no native desktop is created by this platform.
    ui.set_page(2);
    render(&window, &output.join("library-pointer-before.png"))?;
    ui.invoke_toggle_library_item("excluded".into());
    ui.invoke_library_action("keep".into());
    wait(&events, "bulk toolbar keeps excluded phrase", || {
        ui.get_library_items()
            .iter()
            .any(|r| r.identifier == "excluded" && r.kept && !r.excluded)
    })?;
    ui.invoke_toggle_library_item("excluded".into());
    ui.invoke_library_action("exclude".into());
    wait(&events, "bulk toolbar excludes again", || {
        ui.get_library_items()
            .iter()
            .any(|r| r.identifier == "excluded" && r.excluded)
    })?;
    ui.set_page(3);
    ui.set_scroll_y(0.);
    render(&window, &output.join("boolean-pointer-before.png"))?;
    let before = ui.get_text_enabled();
    click(&ui, 406., 148.);
    pump(&events);
    if ui.get_text_enabled() == before {
        return Err("Aligned checkbox pointer did not toggle".into());
    }
    key(&ui, " ");
    pump(&events);
    if ui.get_text_enabled() != before {
        return Err("Aligned checkbox keyboard did not restore state".into());
    }
    // Discard only our synthetic draft, then verify a clean model preparation.
    ui.invoke_discard();
    wait(&events, "discard synthetic checkbox draft", || {
        !ui.get_dirty()
    })?;
    ui.invoke_install_models();
    wait(&events, "basic mode one-click preparation", || {
        ui.get_library_status().contains("Базовый режим готов")
    })?;
    // Save and reload each animation mode through the real settings callback.
    // Only fixture data is touched; restore original values before layout QA.
    for (mode, play, pause) in [(1, 7, 13), (2, 7, 13), (0, 7, 13), (1, 5, 30)] {
        ui.set_animation_index(mode);
        ui.set_animation(mode != 0);
        ui.set_idle_play_seconds(play);
        ui.set_idle_pause_seconds(pause);
        ui.invoke_edited();
        ui.invoke_save();
        wait(&events, "idle settings save and reload", || {
            !ui.get_busy()
                && !ui.get_dirty()
                && super::prefs(&ui).is_ok_and(|p| {
                    p.reminders.animation == (mode != 0)
                        && (mode == 0 || p.reminders.idle_continuous == (mode == 2))
                        && i32::from(p.reminders.idle_play_seconds) == play
                        && i32::from(p.reminders.idle_pause_seconds) == pause
                })
        })?;
        if ui.get_animation_index() != mode {
            return Err("Animation mode changed while reloading settings".into());
        }
    }
    let mut images = vec![];
    let quick = std::env::var_os("KAVYCHKA_FEEDBACK_QUICK").is_some();
    for scale in if quick { vec![1.] } else { vec![1., 1.5] } {
        ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        for (w, h) in if quick {
            vec![(1100, 780)]
        } else {
            vec![(1100, 780), (720, 540)]
        } {
            window.set_size(slint::PhysicalSize::new(
                (w as f32 * scale) as u32,
                (h as f32 * scale) as u32,
            ));
            for en in [false, true] {
                ui.set_language_index(i32::from(en));
                // Refresh translated list captions from the real service.
                ui.invoke_refresh_library();
                pump(&events);
                for theme in if quick { vec![2] } else { vec![1, 2] } {
                    ui.set_theme_index(theme);
                    for page in [0, 1, 2, 3, 4, 5, 6, 7] {
                        ui.set_page(page);
                        ui.set_scroll_y(0.);
                        for state in ["top", "bottom"] {
                            if state == "bottom" {
                                ui.set_scroll_y(-ui.get_scroll_overflow());
                            }
                            let name = format!(
                                "p{page}-{w}x{h}-dpi{scale}-{}-t{theme}-{state}.png",
                                if en { "en" } else { "ru" }
                            );
                            render(&window, &output.join(&name))?;
                            images.push(name);
                        }
                    }
                    ui.set_page(4);
                    ui.set_preset_management(true);
                    ui.set_scroll_y(0.);
                    let name = format!(
                        "presets-{w}-dpi{scale}-{}-t{theme}.png",
                        if en { "en" } else { "ru" }
                    );
                    render(&window, &output.join(&name))?;
                    images.push(name);
                    ui.set_preset_management(false);
                    ui.set_page(2);
                    ui.set_library_support(true);
                    let name = format!(
                        "support-list-{w}-dpi{scale}-{}-t{theme}.png",
                        if en { "en" } else { "ru" }
                    );
                    render(&window, &output.join(&name))?;
                    images.push(name);
                    ui.set_library_support(false);
                }
            }
        }
    }
    ui.window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1. });
    window.set_size(slint::PhysicalSize::new(1100, 780));
    ui.set_page(2);
    ui.set_library_support(false);
    ui.set_scroll_y(0.);
    ui.set_delete_phrase_id("ready".into());
    render(&window, &output.join("delete-confirmation.png"))?;
    key(&ui, slint::platform::Key::Escape);
    pump(&events);
    if !ui.get_delete_phrase_id().is_empty()
        || !ui
            .get_library_items()
            .iter()
            .any(|r| r.identifier == "ready")
    {
        return Err("Delete cancellation changed the library".into());
    }
    ui.set_delete_phrase_id("ready".into());
    key(&ui, slint::platform::Key::Tab);
    key(&ui, slint::platform::Key::Return);
    wait(&events, "single removal", || {
        ui.get_library_items()
            .iter()
            .all(|r| r.identifier != "ready")
    })?;
    ui.hide()?;
    drop(backend);
    let store = Sandbox::open(&data)?;
    if !store.phrase("ready")?.deleted
        || store.phrase("ready")?.text.is_empty()
        || store.phrases()?.len() != 4
    {
        return Err("Removal lost anti-repeat context".into());
    }
    if store.phrase("played")?.used_at.is_none() || store.phrase("old-unused")?.used_at.is_some() {
        return Err("Browsing changed playback history".into());
    }
    if serde_json::to_value(store.load()?)? != serde_json::to_value(&prefs)? {
        return Err("Read-only UI inspection changed saved settings".into());
    }
    if fs::read_dir(store.root.join("audio"))?.count() != 0 {
        return Err("Support generated audio".into());
    }
    drop(store);
    drop(ui);
    // Render the actual companion scene, including proportional cage/timer,
    // with a synthetic pixel sprite. No native desktop window is created.
    let pet = crate::PetWindow::new()?;
    let mut sprite = image::RgbaImage::new(64, 64);
    for y in 12..58 {
        for x in 13..51 {
            sprite.put_pixel(
                x,
                y,
                image::Rgba(if (x / 4 + y / 4) % 2 == 0 {
                    [120, 160, 220, 255]
                } else {
                    [230, 200, 180, 255]
                }),
            );
        }
    }
    let frames = crate::visuals::Frames {
        frames: vec![(sprite, 1000)],
        atlas: false,
        pixelated: true,
    };
    pet.set_dark(true);
    pet.set_custom_visual(true);
    pet.set_custom_pixelated(true);
    pet.set_countdown("14:59".into());
    pet.show()?;
    for scale in [1., 1.25, 1.5] {
        pet.window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        for size in [80, 120, 137, 200, 260] {
            let physical = (size as f32 * scale).round() as u32;
            for paused in [false, true] {
                let height = (crate::visuals::Frames::window_height(size as f32, paused) * scale)
                    .round() as u32;
                window.set_size(slint::PhysicalSize::new(physical, height));
                let p = frames.scene_placement(0, physical, height, scale, paused);
                pet.set_custom_image(frames.image_at(0, p));
                pet.set_custom_left(p.left as f32 / scale);
                pet.set_custom_top(p.top as f32 / scale);
                pet.set_custom_width(p.width as f32 / scale);
                pet.set_custom_height(p.height as f32 / scale);
                pet.set_paused(paused);
                let name = format!("mascot-{size}-dpi{scale}-paused{paused}.png");
                render(&window, &output.join(&name))?;
                images.push(name);
            }
        }
    }
    pet.hide()?;
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?).map_err(|e|e.to_string())?,"software_window":true,"native_windows":false,"desktop_input":false,"audio_played":false,"real_pointer_keyboard":true,"tooltip_snapshot":false,"boolean_pointer_keyboard":true,"basic_model_setup":true,"delete_confirmation_keyboard":true,"library_states":true,"separate_history":true,"exclude_keep":true,"single_delete_keeps_context":true,"settings_preserved":true,"support_text_only":true,"snapshots":images}),
        )?,
    )?;
    Ok(())
}
