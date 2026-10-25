//! Own-window acceptance: actual callbacks/service worker, synthetic data only.
use crate::{AppWindow, models_library::View, service::Backend};
use kavychka_storage::{
    library::Phrase,
    presets::{self, Content, Preset, Reference},
    sandbox::Sandbox,
};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
type Result<T> = kavychka_maintenance::Result<T>;
fn wave(samples: usize, noise: bool) -> Vec<u8> {
    let mut b = vec![];
    let length = (samples * 2) as u32;
    b.extend(b"RIFF");
    b.extend((36 + length).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(length.to_le_bytes());
    let mut state = 0x184abc21u32;
    for i in 0..samples {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let s = if noise {
            state as i16
        } else {
            ((i as f64 * 0.12).sin() * 1200.) as i16
        };
        b.extend(s.to_le_bytes());
    }
    b
}
fn phrase(id: &str) -> Phrase {
    Phrase {
        id: id.into(),
        text: format!("Синтетическая проверка профиля: {id}."),
        category: "break".into(),
        model: "system".into(),
        profile: "system".into(),
        day: Some(chrono::Local::now().date_naive().to_string()),
        audio_path: None,
        approved: false,
        deleted: false,
        rejected: false,
        used_at: None,
        created_at: "2026-10-24T09:00:00Z".into(),
        followup: false,
        language: "ru".into(),
        seed: 187,
        voice_style: "calm".into(),
        voice_instruction: "Calmly".into(),
        voice_reference: String::new(),
    }
}
fn source(root: &Path, flac: &Path) -> Result<(PathBuf, String, String, String)> {
    let data = root.join("source");
    let mut store = Sandbox::open(&data)?;
    let assets = root.join("synthetic-assets");
    fs::create_dir_all(&assets)?;
    fs::write(assets.join("ref.wav"), wave(16000, false))?;
    image::RgbaImage::from_pixel(32, 32, image::Rgba([210, 120, 70, 255]))
        .save(assets.join("visual.png"))?;
    let character = presets::save_character(
        &data,
        "",
        "QA character".into(),
        "Нейтральный синтетический характер.".into(),
    )?;
    let voice = presets::import_prepared(
        &data,
        &Preset {
            version: 1,
            name: "QA voice".into(),
            content: Content::Voice {
                references: vec![Reference {
                    file: "ref.wav".into(),
                    transcript: String::new(),
                }],
            },
        },
        Some(&assets),
    )?;
    let visual = presets::import_prepared(
        &data,
        &Preset {
            version: 1,
            name: "QA visual".into(),
            content: Content::Visual {
                file: "visual.png".into(),
                layout: "image".into(),
            },
        },
        Some(&assets),
    )?;
    let mut prefs = store.load()?;
    prefs.user_name = "QA profile owner".into();
    prefs.desktop.setup_complete = true;
    prefs.desktop.character_preset = character.clone();
    prefs.desktop.visual_preset = visual.clone();
    prefs.generation.voice_profile = voice.clone();
    prefs.generation.auto_daily = false;
    prefs.schedule.work_days.clear();
    store.save(&prefs)?;
    let stage = root.join("audio-stage");
    fs::create_dir_all(stage.join("audio"))?;
    fs::copy(flac, stage.join("audio/tone.flac"))?;
    for id in ["same", "unused", "excluded", "history"] {
        let mut row = phrase(id);
        row.profile = voice.clone();
        row.audio_path = Some("tone.flac".into());
        row.approved = true;
        if id == "same" {
            row.used_at = Some("2026-10-24T10:00:00Z".into());
        }
        if id == "excluded" {
            row.rejected = true;
        }
        if id == "history" {
            row.deleted = true;
        }
        store.accept_phrase(&row, Some(&stage))?;
    }
    // Valid but unreferenced generated PCM makes the worker observable without
    // relying on artificial sleeps. It must survive the audio-inclusive archive.
    fs::write(data.join("audio/bulk.wav"), wave(6_000_000, true))?;
    store.put_meta(
        "audio_day",
        &serde_json::json!(chrono::Local::now().date_naive().to_string()),
    )?;
    Ok((data, character, voice, visual))
}
fn snapshot(ui: &AppWindow, path: &Path) -> Result<()> {
    let image = ui.window().take_snapshot()?;
    image::save_buffer(
        path,
        image.as_bytes(),
        image.width(),
        image.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
pub(crate) fn click(ui: &AppWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
fn attach(ui: &AppWindow, backend: &Rc<RefCell<Option<Backend>>>, path: &Path) {
    backend.borrow_mut().take();
    ui.set_ready(false);
    ui.set_dirty(false);
    ui.set_profile_review("".into());
    ui.set_profile_status_ru("".into());
    ui.set_profile_output("".into());
    ui.set_profile_running(false);
    ui.set_data_operation_active(false);
    ui.set_library_json("".into());
    ui.set_storage_path(path.to_string_lossy().as_ref().into());
    let next = Backend::start(path.to_owned(), ui.as_weak());
    crate::wire(ui, &next);
    *backend.borrow_mut() = Some(next);
}
fn nonempty(root: &Path, source: &Path, character: &str) -> Result<PathBuf> {
    let data = root.join("nonempty");
    let mut store = Sandbox::open(&data)?;
    let folder = presets::folder(&data, character)?;
    fs::create_dir_all(&folder)?;
    fs::copy(
        presets::folder(source, character)?.join("preset.json"),
        folder.join("preset.json"),
    )?;
    presets::save_character(
        &data,
        character,
        "Current character".into(),
        "Сохранить текущий характер.".into(),
    )?;
    let mut prefs = store.load()?;
    prefs.user_name = "Current owner".into();
    prefs.desktop.setup_complete = true;
    prefs.desktop.character_preset = character.into();
    prefs.generation.auto_daily = false;
    store.save(&prefs)?;
    let mut current = phrase("same");
    current.text = "Сохранить текущий выбор пользователя.".into();
    current.rejected = true;
    store.accept_phrase(&current, None)?;
    Ok(data)
}
pub fn run(evidence: PathBuf, flac: PathBuf) -> Result<()> {
    if !evidence.is_absolute() || evidence.exists() {
        return Err("New absolute QA evidence directory required".into());
    }
    if !fs::read(&flac)?.starts_with(b"fLaC") {
        return Err("Synthetic FLAC fixture required".into());
    }
    fs::create_dir_all(&evidence)?;
    fs::write(evidence.join("p5-qa.owner"), b"Kavychka.P5.synthetic.v1\n")?;
    let (source, character, voice, visual) = source(&evidence, &flac)?;
    let clean_common = evidence.join("clean-common");
    let clean = clean_common.join(".kavychka-next");
    {
        let mut fresh = Sandbox::open(&clean)?;
        fresh.set_recovery_limit(100_000_000)?;
    }
    crate::setup::prepare(&clean, &clean_common)?;
    {
        let fresh = Sandbox::open(&clean)?;
        if fresh.has_preferences()? || fresh.load()?.desktop.recovery_limit_bytes != 100_000_000 {
            return Err("Installer consumed clean profile or lost chosen budget".into());
        }
    }
    let archive = evidence.join("profile.zip");
    let source_audio = kavychka_maintenance::digest(&source.join("audio/tone.flac"))?;
    crate::shell::select_backend(false)?;
    let ui = AppWindow::new()?;
    ui.set_app_version(crate::app_version().into());
    let backend = Rc::new(RefCell::new(None));
    attach(&ui, &backend, &source);
    let outcome = Rc::new(RefCell::new(None::<std::result::Result<(), String>>));
    let result = outcome.clone();
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let owned = backend.clone();
    let output = evidence.clone();
    let mut phase = 0;
    let mut ticks = 0u64;
    let mut busy_ticks = 0u64;
    let mut before = String::new();
    let mut frames = 0;
    let started = Instant::now();
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(100),move||{
        let Some(ui)=weak.upgrade() else{return;};ticks+=1;
        let check=(||->Result<()>{
            if started.elapsed()>Duration::from_secs(240){return Err(format!("P5 UI timeout phase {phase}: {}",ui.get_profile_status_ru()).into());}
            if ui.get_failed(){return Err(ui.get_status().to_string().into());}
            if ui.get_profile_status_ru().starts_with("Операция с данными не выполнена"){return Err(ui.get_profile_status_ru().to_string().into());}
            if ui.get_profile_running(){busy_ticks+=1;ui.set_page(if ticks.is_multiple_of(2){0}else{6});}
            if !ui.get_ready()||ui.get_library_json().is_empty(){return Ok(());}
            let view:View=serde_json::from_str(ui.get_library_json().as_str())?;
            match phase {
                0=>{ui.set_page(6);ui.set_profile_export_path(archive.to_string_lossy().as_ref().into());ui.invoke_profile_action(0);phase=1;}
                1 if ui.get_profile_review()=="export"&&!ui.get_profile_running()=>{before=kavychka_maintenance::digest(&source.join("shell.sqlite3"))?;snapshot(&ui,&output.join("export-review-ru-light.png"))?;ui.invoke_profile_action(1);phase=2;}
                2 if archive.is_file()&&!ui.get_profile_running()=>{if busy_ticks<2{return Err("UI worker responsiveness was not observed".into());}owned.borrow_mut().take();if kavychka_maintenance::digest(&source.join("shell.sqlite3"))?!=before{return Err("Source DB changed during export".into());}attach(&ui,&owned,&clean);phase=3;}
                3=>{if !ui.get_onboarding()||!view.phrases.is_empty(){return Err("Clean restore entry is not before setup".into());}ui.set_profile_operation_index(1);ui.set_profile_import_path(archive.to_string_lossy().as_ref().into());ui.invoke_profile_action(2);phase=4;}
                4 if ui.get_profile_can_apply()&&!ui.get_profile_running()=>{if ui.get_user_name()=="QA profile owner"||!view.phrases.is_empty(){return Err("Inspect modified clean settings/library".into());}snapshot(&ui,&output.join("welcome-restore-review.png"))?;ui.set_onboarding(false);ui.set_page(6);ui.window().set_size(slint::LogicalSize::new(720.,480.));phase=5;}
                5=>{let english=frames>=2;let dark=frames%2==1;ui.set_language_index(i32::from(english));ui.set_theme_index(if dark{2}else{1});ui.set_scroll_y(-ui.get_scroll_overflow()*0.5);phase=6;}
                6=>{snapshot(&ui,&output.join(format!("restore-review-{}-{}.png",if frames>=2{"en"}else{"ru"},if frames%2==1{"dark"}else{"light"})))?;frames+=1;if frames<4{phase=5;}else{ui.invoke_profile_action(3);phase=7;}}
                7 if !ui.get_profile_running()&&ui.get_profile_status_ru().starts_with("Восстановление завершено")=>{
                    if ui.get_user_name()!="QA profile owner"||ui.get_onboarding(){return Err("Clean restore did not apply prefs/setup".into());}
                    if view.phrases.len()!=4{return Ok(());}
                    owned.borrow_mut().take();let store=Sandbox::open(&clean)?;let prefs=store.load()?;
                    if prefs.desktop.character_preset!=character||prefs.generation.voice_profile!=voice||prefs.desktop.visual_preset!=visual{return Err("Independent preset links changed".into());}
                    if !store.phrase("excluded")?.rejected||!store.phrase("history")?.deleted||store.phrase("same")?.used_at.is_none(){return Err("History/status lost".into());}
                    if kavychka_maintenance::digest(&clean.join("audio/tone.flac"))?!=source_audio{return Err("Restored audio changed".into());}drop(store);
                    attach(&ui,&owned,&clean);phase=8;
                }
                8=>{ui.set_profile_import_path(archive.to_string_lossy().as_ref().into());ui.invoke_profile_action(2);phase=9;}
                9 if ui.get_profile_can_apply()&&!ui.get_profile_running()=>{if !ui.get_profile_summary_ru().contains("уже применён"){return Err("Repeat not identified".into());}ui.set_page(6);ui.window().set_size(slint::LogicalSize::new(1100.,780.));phase=90;}
                90=>{ui.set_scroll_y(-ui.get_scroll_overflow());phase=91;}
                91=>{snapshot(&ui,&output.join("repeat-review-actions.png"))?;if ui.get_busy()||ui.get_onboarding(){return Err("Restore review parent stayed disabled".into());}click(&ui,860.,475.);phase=10;}
                10 if !ui.get_data_operation_active()=>{if view.phrases.len()!=4{return Err("Repeat/cancel changed rows".into());}
                    ui.invoke_profile_action(2);phase=11;
                }
                11 if ui.get_profile_can_apply()&&!ui.get_profile_running()=>{ui.set_page(6);phase=111;}
                111=>{ui.set_scroll_y(-ui.get_scroll_overflow());phase=110;}
                110=>{snapshot(&ui,&output.join("repeat-apply-actions.png"))?;click(&ui,464.,475.);phase=12;}
                12 if !ui.get_profile_running()&&ui.get_profile_status_ru().starts_with("Восстановление завершено")=>{
                    if view.phrases.len()!=4{return Err("Repeat apply duplicated rows".into());}
                    let path=nonempty(&output,&source,&character)?;attach(&ui,&owned,&path);phase=13;
                }
                13=>{ui.set_profile_operation_index(1);ui.set_profile_import_path(archive.to_string_lossy().as_ref().into());ui.invoke_profile_action(2);phase=14;}
                14 if ui.get_profile_can_apply()&&!ui.get_profile_running()=>{
                    if ui.get_user_name()!="Current owner"||view.phrases.len()!=1||!ui.get_profile_summary_ru().contains("Текущие настройки"){return Err("Nonempty preview changed current data or policy".into());}
                    ui.set_page(6);ui.set_scroll_y(-ui.get_scroll_overflow()*0.7);snapshot(&ui,&output.join("nonempty-review.png"))?;ui.invoke_profile_action(3);phase=15;
                }
                15 if !ui.get_profile_running()&&ui.get_profile_status_ru().starts_with("Восстановление завершено")=>{
                    if view.phrases.len()!=4{return Ok(());}
                    owned.borrow_mut().take();let current=Sandbox::open(&output.join("nonempty"))?;
                    if current.load()?.user_name!="Current owner"||current.load()?.desktop.character_preset!=character||!current.phrase("same")?.rejected||current.phrase("same")?.text!="Сохранить текущий выбор пользователя."{return Err("Nonempty apply overwrote current choices".into());}
                    let presets=presets::list(&current.root)?;if !presets.iter().any(|p|p.id!=character&&p.preset.name=="QA character"){return Err("Conflicting incoming preset was lost".into());}drop(current);
                    attach(&ui,&owned,&clean);phase=16;
                }
                16=>{ui.set_page(6);ui.set_profile_operation_index(0);ui.set_profile_export_path(output.join("cancel-worker.zip").to_string_lossy().as_ref().into());ui.invoke_profile_action(0);phase=17;}
                17 if ui.get_profile_review()=="export"&&!ui.get_profile_running()=>{ui.invoke_profile_action(1);phase=18;}
                18 if ui.get_profile_running()=>{ui.invoke_profile_action(4);phase=19;}
                19 if !ui.get_profile_running()=>{
                    let cancelled=output.join("cancel-worker.zip");
                    if cancelled.exists(){
                        if !ui.get_profile_status_ru().starts_with("Архив проверен"){return Err("Late cancellation hid an already committed export".into());}
                    }else if !ui.get_profile_status_ru().contains("отменена"){return Err("Worker cancellation was not acknowledged".into());}
                    ui.set_profile_export_path(output.join("exit-worker.zip").to_string_lossy().as_ref().into());ui.invoke_profile_action(0);phase=20;
                }
                20 if ui.get_profile_review()=="export"&&!ui.get_profile_running()=>{ui.invoke_profile_action(1);phase=21;}
                21 if ui.get_profile_running()=>{
                    owned.borrow_mut().take();
                    if fs::read_dir(&output)?.any(|p|p.is_ok_and(|p|p.path().extension().is_some_and(|e|e=="part"))){return Err("Graceful service exit left partial ZIP".into());}
                    let check=Sandbox::open(&clean)?;if check.phrases()?.len()!=4{return Err("Exit changed source rows".into());}drop(check);
                    kavychka_maintenance::atomic(&output.join("result.json"),&serde_json::json!({"passed":true,"native_ui_callbacks":true,"pointer_cancel_and_apply_during_review":true,"settings_mode_no_python":true,"export_verified":true,"source_database_unchanged":true,"restore_before_setup":true,"preview_does_not_apply":true,"explicit_restore":true,"independent_presets":true,"history_exclusions_audio_hashes":true,"nonempty_merge_and_preset_conflict":true,"repeat_identified_cancelled_and_applied_noop":true,"worker_cancel_and_graceful_service_exit":true,"background_ui_ticks":busy_ticks,"total_ui_ticks":ticks,"frames":frames+5,"exe_sha256":kavychka_maintenance::digest(&std::env::current_exe()?)?,"physical_P4_checks":false}))?;phase=22;
                }
                _=>{}
            }
            Ok(())
        })();
        if check.is_err()||phase==22{*result.borrow_mut()=Some(check.map_err(|e|e.to_string()));let _=slint::quit_event_loop();}
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    backend.borrow_mut().take();
    outcome
        .borrow_mut()
        .take()
        .ok_or("P5 native UI did not finish")?
        .map_err(Into::into)
}
