//! P3 acceptance on this application's own windows and disposable storage.
//! No desktop input, user library, ML models or Python process is required.
use crate::*;
use slint::platform::WindowEvent;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
pub fn run(data: PathBuf, output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if data.exists() {
        return Err("P3 acceptance requires a new data folder".into());
    }
    std::fs::create_dir_all(&output)?;
    {
        let mut store = Sandbox::open(&data)?;
        let mut p = store.load()?;
        p.schedule.work_days.clear(); // No scheduled desktop notification during QA.
        p.generation.auto_daily = false;
        p.desktop.visual_preset = "p3visual".into();
        p.desktop.automatic_updates = false;
        p.reminders.fullscreen_visual = VisualMode::Both;
        p.reminders.audio = AudioMode::Silent;
        store.save(&p)?;
        let folder = data.join("presets/p3visual");
        std::fs::create_dir_all(&folder)?;
        let img = image::RgbaImage::from_fn(100, 100, |x, y| {
            image::Rgba([
                70,
                90,
                200,
                if (30..70).contains(&x) && (30..70).contains(&y) {
                    0
                } else {
                    255
                },
            ])
        });
        img.save(folder.join("ring.png"))?;
        std::fs::write(
            folder.join("preset.json"),
            serde_json::to_vec(
                &serde_json::json!({"version":1,"kind":"visual","name":"P3 synthetic ring","file":"ring.png","layout":"image"}),
            )?,
        )?;
    }
    shell::select_backend(true)?;
    let ui = AppWindow::new()?;
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start_runtime(data.clone(), ui.as_weak());
    wire(&ui, &backend);
    let companion = shell::Shell::new(&ui, backend.sender.clone())?;
    companion.activation.set(false);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let pet = companion.clone();
    let evidence = output.clone();
    let result = Rc::new(RefCell::new(None));
    let outcome = result.clone();
    let started = Instant::now();
    let mut phase = 0;
    let mut phase_since = Instant::now();
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(100),move|| {
        let Some(ui)=weak.upgrade() else { return; };
        let pass=(||->Result<(),String> {
            if started.elapsed()>Duration::from_secs(35) { return Err(format!("P3 timeout phase {phase}: {}; audio={}; visual={}; visible={}; view={:?}",ui.get_status(),ui.get_audio_test_status(),pet.pet.get_custom_visual(),pet.pet.window().is_visible(),pet.view.borrow().as_ref().map(|v|(v.prefs.desktop.visual_preset.clone(),v.pet_permitted,v.status.clone())))); }
            if ui.get_failed() { return Err(ui.get_status().to_string()); }
            match phase {
                0 if ui.get_ready() && phase_since.elapsed()>Duration::from_secs(1)=>{
                    if !ui.get_onboarding() || pet.view.borrow().is_some() || pet.pet.window().is_visible() || pet.message.window().is_visible() || ui.get_generation_running() { return Err("unconfigured startup ran reminders/models".into()); }
                    // A fresh --background launch must still expose setup; no explicit show below.
                    if !ui.window().is_visible() { return Err("first launch hidden in background".into()); }
                    ui.set_language_index(1); ui.set_speech_index(0); ui.set_address_index(2); ui.invoke_edited();
                    ui.invoke_setup_next(1); ui.invoke_setup_next(1);
                    ui.invoke_test_audio(0); phase=1;
                }
                1 if ui.get_audio_test_status()=="Sound test completed"=>{
                    let p:Preferences=serde_json::from_str(ui.get_loaded_prefs().as_str()).map_err(|e|e.to_string())?;
                    if p.desktop.setup_complete || !p.user_name.is_empty() || pet.view.borrow().is_some() { return Err("sound preview committed setup or started reminders".into()); }
                    ui.invoke_setup_next(1); ui.invoke_finish_setup(true); phase=2;
                }
                2 if !ui.get_busy() && !ui.get_onboarding() && pet.pet.get_custom_visual() && pet.pet.window().is_visible()=>{
                    if ui.get_page()!=3 { return Err("optional model route lost".into()); }
                    let hwnd=shell::handle(pet.pet.window()).ok_or("missing pet HWND")?;
                    let size=pet.pet.window().size(); let scale=pet.pet.window().scale_factor();
                    let contains=|x:i32,y:i32| kavychka_platform::desktop::region_contains(hwnd,x,y);
                    if contains((size.width/2) as i32,(size.height/2) as i32)!=Some(false) || contains((20.*scale) as i32,(20.*scale) as i32)!=Some(true) || contains(0,0)!=Some(false) { return Err(format!("native current-frame alpha region wrong: center={:?}, edge={:?}, corner={:?}, size={size:?}",contains((size.width/2) as i32,(size.height/2) as i32),contains((20.*scale) as i32,(20.*scale) as i32),contains(0,0))); }
                    let p=pet.pet.window().take_snapshot().map_err(|e|e.to_string())?;
                    image::save_buffer(evidence.join("alpha-pet.png"),p.as_bytes(),p.width(),p.height(),image::ColorType::Rgba8).map_err(|e|e.to_string())?;
                    // Resizing must recompute physical alpha spans.
                    ui.set_pet_size(200); ui.set_animation_index(0); ui.invoke_edited(); ui.invoke_save(); phase_since=Instant::now(); phase=3;
                }
                3 if !ui.get_busy() && pet.pet.window().size().width >= 200 && phase_since.elapsed()>Duration::from_millis(400)=>{
                    let hwnd=shell::handle(pet.pet.window()).ok_or("missing resized HWND")?;
                    let size=pet.pet.window().size();
                    if kavychka_platform::desktop::region_contains(hwnd,(size.width/2) as i32,(size.height/2) as i32)!=Some(false) { return Err("resize filled transparent hole".into()); }
                    ui.set_user_name("Unsaved keyboard test".into()); ui.invoke_edited(); ui.window().dispatch_event(WindowEvent::CloseRequested);
                    if !ui.get_confirm_close() { return Err("OS close lost dirty guard".into()); }
                    ui.set_confirm_close(false); ui.invoke_discard(); phase=4;
                }
                4 if !ui.get_busy()=>{
                    ui.window().dispatch_event(WindowEvent::CloseRequested); phase=5;
                }
                5 if !ui.window().is_visible()=>{
                    if !kavychka_platform::desktop::tray_registered() { return Err("background closed tray".into()); }
                    pet.open(); phase=6;
                }
                6 if ui.window().is_visible()=>{
                    if ui.get_onboarding() || ui.get_dirty() { return Err("reopen repeated setup/lost discard".into()); }
                    *outcome.borrow_mut()=Some(Ok(())); pet.tray().invoke_quit();
                }
                _=>{}
            }
            if phase==0 { phase_since=started; }
            Ok(())
        })();
        if let Err(e)=pass { *outcome.borrow_mut()=Some(Err(e)); let _=slint::quit_event_loop(); }
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    drop(timer);
    ui.hide()?;
    drop(companion);
    drop(backend);
    result
        .borrow_mut()
        .take()
        .ok_or("P3 loop stopped early")?
        .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
    if kavychka_platform::desktop::tray_window() != 0 {
        return Err("P3 tray leaked on exit".into());
    }
    let store = Sandbox::open(&data)?;
    let p = store.load()?;
    if !p.desktop.setup_complete
        || p.generation.address_form != "male"
        || p.desktop.pet_size != 200
        || p.reminders.animation_intensity != AnimationIntensity::Off
    {
        return Err("P3 restart persistence failed".into());
    }
    if !store.load_state()?.handled.is_empty() {
        return Err("setup preview consumed reminders".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"native_winit":true,"fresh_background_setup_visible":true,"unconfigured_no_reminders_or_generation":true,"explicit_chime_without_saving_draft":true,"optional_models_route":true,"native_current_frame_alpha_hole":true,"resize_alpha":true,"close_guard_discard_background_reopen":true,"saved_setup_address_animation_size":true,"clean_exit_tray_released":true,"python_or_models_used":false,"desktop_input_injected":false,"live_storage_opened":false,"physical_monitor_dpi_tested":false,"narrator_tested":false}),
        )?,
    )?;
    println!("P3 native shell acceptance passed.");
    Ok(())
}
