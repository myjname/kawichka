//! Real batch callbacks and SQLite on isolated data, without native input/audio/models.
use super::*;
use serde_json::json;

fn verify_details_overlay(
    ui: &AppWindow,
    window: &MinimalSoftwareWindow,
    events: &Events,
    output: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let before = output.join(format!("{name}-before-details.png"));
    let details = output.join(format!("{name}-details.png"));
    render(window, &before)?;
    let selected = ui.get_library_selected_count();
    ui.set_task_title("TASK DETAILS ABOVE LIBRARY".into());
    ui.set_task_details("Progress and diagnostic details must cover the library toolbar.".into());
    ui.set_show_task_details(true);
    pump(events);
    render(window, &details)?;
    let before = image::open(before)?.to_rgba8();
    let details = image::open(details)?.to_rgba8();
    if image::imageops::crop_imm(&before, 226, 96, 830, 100).to_image()
        == image::imageops::crop_imm(&details, 226, 96, 830, 100).to_image()
    {
        return Err("library toolbar covered task details".into());
    }
    click(ui, 232., 230.);
    pump(events);
    if ui.get_library_selected_count() != selected {
        return Err("task details click selected a phrase underneath".into());
    }
    click(ui, 260., 368.);
    wait(events, "task details close button", || {
        !ui.get_show_task_details()
    })?;
    Ok(())
}

pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("new evidence folder required".into());
    }
    fs::create_dir(&output)?;
    let data = output.join("data");
    let mut store = Sandbox::open(&data)?;
    let mut original = store.load()?;
    original.desktop.setup_complete = true;
    original.desktop.automatic_updates = false;
    original.generation.auto_daily = false;
    original.generation.text_enabled = false;
    original.generation.tts_model = "system".into();
    original.generation.ac_only = false;
    store.save(&original)?;
    let day = kavychka_platform::time::now_in(&original.desktop.time_zone)?.date;
    for i in 0..73 {
        let date = if i < 65 {
            day
        } else {
            day - chrono::Days::new(1)
        };
        let row = serde_json::from_value::<kavychka_storage::library::Phrase>(
            json!({"id":format!("row{i:03}"),"text":format!("Фраза номер {i}. Проверяем выбор строк, читаемость текста и неподвижную панель действий при прокрутке. Вторая строка не должна пересекаться с разделителем."),"category":"break","model":"fixture-voice","profile":"fixture","day":date.to_string(),"audio_path":null,"approved":true,"deleted":false,"rejected":false,"used_at":null,"created_at":date.and_hms_opt(12,0,0).unwrap().and_utc().to_rfc3339(),"followup":false,"language":"ru"}),
        )?;
        store.accept_phrase(&row, None)?;
    }
    drop(store);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let events = Events::default();
    slint::platform::set_platform(Box::new(Headless {
        window: window.clone(),
        events: events.clone(),
    }))?;
    let ui = AppWindow::new()?;
    ui.set_audio_refreshing(true);
    ui.set_storage_path(data.to_string_lossy().as_ref().into());
    let backend = Backend::start(data.clone(), ui.as_weak());
    crate::wire(&ui, &backend);
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1100, 780));
    ui.set_page(2);
    wait(&events, "library rows", || {
        ui.get_ready()
            && ui.get_library_current_count() == 65
            && ui.get_library_items().row_count() == 50
            && ui.get_support_texts().row_count() == 4
    })?;
    render(&window, &output.join("current-top.png"))?;
    verify_details_overlay(&ui, &window, &events, &output, "current")?;
    let before_double_click = crate::library_actions::selection(&ui).revision;
    click(&ui, 232., 184.);
    click(&ui, 232., 184.);
    if crate::library_actions::selection(&ui).revision == before_double_click {
        return Err("rapid All double click did not cancel the pending selection".into());
    }
    pump(&events);
    if ui.get_library_selected_count() != 0 {
        return Err("cancelled All double click selected phrases".into());
    }
    click(&ui, 232., 230.);
    wait(&events, "row checkbox pointer selection", || {
        ui.get_library_selected_count() == 1
    })?;
    let first = ui.get_library_items().row_data(0).unwrap().identifier;
    let old_action = crate::library_actions::selection(&ui);
    ui.invoke_toggle_library_item(first.clone());
    ui.invoke_toggle_library_item(first.clone());
    crate::library_actions::clear_if_unchanged(&ui, &old_action);
    if ui.get_library_selected_count() != 1 {
        return Err("late action cleared a newly restored selection".into());
    }
    key(&ui, " ");
    wait(&events, "row checkbox keyboard deselection", || {
        ui.get_library_selected_count() == 0
    })?;
    let old_all = crate::library_actions::selection(&ui);
    ui.invoke_toggle_library_item(first.clone());
    ui.invoke_toggle_library_item(first.clone());
    crate::library_actions::accept_all(&ui, &old_all, vec![first.to_string()]);
    if ui.get_library_selected_count() != 0 {
        return Err("late select-all restored cancelled selection".into());
    }
    ui.set_library_scroll_y(-ui.get_library_scroll_overflow());
    render(&window, &output.join("current-scrolled.png"))?;
    if ui.get_scroll_y() != 0. || ui.get_library_scroll_y() >= 0. {
        return Err("library did not scroll independently".into());
    }
    let top = image::open(output.join("current-top.png"))?.to_rgba8();
    let scrolled = image::open(output.join("current-scrolled.png"))?.to_rgba8();
    if image::imageops::crop_imm(&top, 210, 80, 870, 132).to_image()
        != image::imageops::crop_imm(&scrolled, 210, 80, 870, 132).to_image()
    {
        return Err("toolbar moved with list".into());
    }
    ui.invoke_select_all_library(true);
    wait(&events, "select all across pages", || {
        ui.get_library_selected_count() == 65
    })?;
    ui.invoke_library_page(1);
    wait(&events, "second page selected", || {
        ui.get_library_page_index() == 1
            && ui.get_library_items().row_count() == 15
            && ui.get_library_items().iter().all(|r| r.selected)
    })?;
    let last = ui.get_library_items().row_data(0).unwrap().identifier;
    ui.invoke_toggle_library_item(last);
    if ui.get_library_selected_count() != 64 {
        return Err("single deselection failed".into());
    }
    ui.invoke_library_page(-1);
    wait(&events, "first page selection survives", || {
        ui.get_library_page_index() == 0 && ui.get_library_items().iter().all(|r| r.selected)
    })?;
    ui.invoke_select_all_library(false);
    let ids = ui
        .get_library_items()
        .iter()
        .take(2)
        .map(|r| r.identifier)
        .collect::<Vec<_>>();
    for id in &ids {
        ui.invoke_toggle_library_item(id.clone());
    }
    ui.invoke_library_action("exclude".into());
    wait(&events, "bulk exclude", || {
        ui.get_library_selected_count() == 0
            && ui
                .get_library_items()
                .iter()
                .filter(|r| ids.contains(&r.identifier))
                .all(|r| r.excluded)
    })?;
    for id in &ids {
        ui.invoke_toggle_library_item(id.clone());
    }
    ui.invoke_library_action("keep".into());
    wait(&events, "bulk keep", || {
        ui.get_library_selected_count() == 0
            && ui
                .get_library_items()
                .iter()
                .filter(|r| ids.contains(&r.identifier))
                .all(|r| r.kept && !r.excluded)
    })?;
    for id in &ids {
        ui.invoke_toggle_library_item(id.clone());
    }
    ui.invoke_library_action("speech".into());
    wait(&events, "only selected speech rows", || {
        ui.get_library_selected_count() == 0
            && ui
                .get_library_items()
                .iter()
                .filter(|r| ids.contains(&r.identifier))
                .all(|r| r.caption.contains("Системный голос"))
    })?;
    ui.set_library_today(false);
    ui.invoke_library_filter();
    wait(&events, "history", || {
        ui.get_library_items().row_count() == 8 && ui.get_library_selected_count() == 0
    })?;
    verify_details_overlay(&ui, &window, &events, &output, "history")?;
    ui.invoke_select_all_library(true);
    wait(&events, "history select all", || {
        ui.get_library_selected_count() == 8
    })?;
    ui.invoke_library_action("speech".into());
    pump(&events);
    if !ui.get_library_status().contains("Действие недоступно") {
        return Err("history allowed speech".into());
    }
    render(&window, &output.join("history.png"))?;
    ui.set_library_delete_selection(ui.get_library_selection());
    ui.set_confirm_library_delete(true);
    key(&ui, slint::platform::Key::Escape);
    pump(&events);
    if ui.get_confirm_library_delete() || ui.get_library_selected_count() != 8 {
        return Err("delete cancel lost selection".into());
    }
    ui.set_confirm_library_delete(true);
    key(&ui, slint::platform::Key::Tab);
    key(&ui, slint::platform::Key::Return);
    wait(&events, "bulk history deletion", || {
        ui.get_library_history_count() == 0 && ui.get_library_items().row_count() == 0
    })?;
    ui.set_library_support(true);
    ui.invoke_library_filter();
    wait(&events, "support rows before selection", || {
        ui.get_support_items().row_count() == 4 && ui.get_library_selected_count() == 0
    })?;
    click(&ui, 232., 230.);
    wait(&events, "support pointer selection", || {
        ui.get_library_selected_count() == 1
    })?;
    let support_model = ui.get_support_items();
    crate::models_ui::render(&ui, ui.get_library_json().as_str())?;
    if !std::ptr::eq(support_model.as_any(), ui.get_support_items().as_any()) {
        return Err("support refresh recreated focused checkbox delegates".into());
    }
    key(&ui, " ");
    wait(
        &events,
        "support keyboard selection survives refresh",
        || ui.get_library_selected_count() == 0,
    )?;
    ui.invoke_select_all_library(true);
    wait(&events, "support select all", || {
        ui.get_library_selected_count() == 4
    })?;
    render(&window, &output.join("support.png"))?;
    ui.invoke_library_action("delete".into());
    wait(&events, "support deletion", || {
        ui.get_support_texts().row_count() == 0 && ui.get_library_selected_count() == 0
    })?;
    ui.set_library_support(false);
    ui.set_library_today(true);
    ui.invoke_library_filter();
    wait(&events, "current restored", || {
        ui.get_library_items().row_count() == 50
    })?;
    for id in &ids {
        ui.invoke_toggle_library_item(id.clone());
    }
    ui.invoke_library_action("delete".into());
    wait(&events, "delete selected current", || {
        ui.get_library_current_count() == 63
    })?;
    for en in [false, true] {
        ui.set_language_index(i32::from(en));
        ui.invoke_refresh_library();
        pump(&events);
        window.set_size(slint::PhysicalSize::new(720, 540));
        render(
            &window,
            &output.join(if en {
                "compact-en.png"
            } else {
                "compact-ru.png"
            }),
        )?;
    }
    ui.hide()?;
    drop(backend);
    let store = Sandbox::open(&data)?;
    let rows = store.phrases()?;
    if rows.len() != 73
        || rows.iter().filter(|r| r.deleted).count() != 10
        || rows
            .iter()
            .any(|r| r.text.is_empty() || r.used_at.is_some())
    {
        return Err("batch operations lost context or changed playback facts".into());
    }
    if rows.iter().filter(|r| r.model == "system").count() != 2 {
        return Err("speech touched unselected rows".into());
    }
    if !crate::support::load(&store, &original, &day.to_string())?
        .texts
        .is_empty()
    {
        return Err("deleted support returned after reload".into());
    }
    if serde_json::to_value(store.load()?)? != serde_json::to_value(original)? {
        return Err("library changed settings".into());
    }
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"actual_service_and_sqlite":true,"all_65_across_pages":true,"selection_survives_pagination":true,"toolbar_static_pixels_during_scroll":true,"bulk_keep_exclude_revoice_delete":true,"revoice_only_two_selected":true,"history_readonly_except_delete":true,"support_delete_survives_reload":true,"delete_confirmation_cancel_and_keyboard_confirm":true,"anti_repeat_context_preserved":true,"playback_history_unchanged":true,"settings_unchanged":true,"native_windows":false,"desktop_input":false,"models_run":false,"audio_played":false}),
        )?,
    )?;
    Ok(())
}
