use crate::{
    AppWindow, LibraryItem,
    models_library::View,
    service::{Backend, Command},
};
use kavychka_core::{
    generation::{TEXT_MODELS, VOICE_MODELS},
    preferences::Preferences,
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
fn strings(values: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}
fn phrase_status(
    r: &kavychka_storage::library::Phrase,
    view: &View,
) -> (&'static str, &'static str) {
    let en = view.english;
    let (state, ru, english) = match view
        .delivery_states
        .get(&r.id)
        .map(String::as_str)
        .unwrap_or("")
    {
        "audio_completed" => ("✓", "Озвучена в напоминании", "Played in a reminder"),
        "audio_failed" => (
            "!",
            "Ошибка озвучки; фраза уже использована",
            "Speech failed; phrase already used",
        ),
        "audio_cancelled" => (
            "■",
            "Озвучка остановлена; фраза уже использована",
            "Speech stopped; phrase already used",
        ),
        "audio_started" | "audio_preparing" => (
            "…",
            "Озвучка начата; завершение не подтверждено",
            "Speech requested; completion not confirmed",
        ),
        "shown" | "committed" => (
            "◷",
            "Использована в напоминании; завершённая озвучка не подтверждена",
            "Used in a reminder; completed speech not confirmed",
        ),
        _ if r.used_at.is_some() => (
            "◷",
            "Ранее использована; сведений об озвучке нет",
            "Previously used; no playback evidence",
        ),
        _ if r.rejected => ("×", "Исключена из напоминаний", "Excluded from reminders"),
        _ if r.day.as_deref() != Some(&view.day) => (
            "—",
            "Другой день: не используется сегодня",
            "Different day: not used today",
        ),
        _ if !r.approved => (
            "?",
            "Ожидает отбора: нажми «Оставить»",
            "Needs review: choose Keep",
        ),
        _ if !view.audio_available.contains(&r.id) => (
            "!",
            "Нет готовой записи: нужна озвучка",
            "No recording: generate speech",
        ),
        _ => (
            "●",
            "Доступна для сегодняшних напоминаний",
            "Available for today's reminders",
        ),
    };
    (state, if en { english } else { ru })
}
fn local_status(text: &str, en: bool) -> String {
    text.lines()
        .map(|line| {
            if let Some((ru, english)) = line.split_once(" / ") {
                if en {
                    english.to_owned()
                } else if let Some((_, detail)) = english.split_once(" · ") {
                    format!("{ru} · {detail}")
                } else {
                    ru.to_owned()
                }
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn apply(ui: &AppWindow, p: &Preferences) {
    let g = &p.generation;
    ui.set_compute_device(
        ["auto", "cpu", "cuda"]
            .iter()
            .position(|v| *v == g.device)
            .unwrap_or(0) as i32,
    );
    ui.set_text_model_index(
        TEXT_MODELS
            .iter()
            .position(|id| *id == g.text_model)
            .unwrap_or(0) as i32,
    );
    ui.set_runtime_model_index(
        TEXT_MODELS
            .iter()
            .position(|id| *id == g.runtime_text_model)
            .unwrap_or(0) as i32,
    );
    ui.set_voice_model_index(
        VOICE_MODELS
            .iter()
            .position(|id| *id == g.tts_model)
            .unwrap_or(0) as i32,
    );
    ui.set_text_enabled(g.text_enabled);
    ui.set_runtime_generation(g.runtime);
    ui.set_auto_daily(g.auto_daily);
    ui.set_auto_keep(g.daily_auto_keep);
    ui.set_generation_ac_only(g.ac_only);
    ui.set_swear(g.swear);
    ui.set_batch_count(g.batch_count.into());
    ui.set_category_break(g.categories.iter().any(|c| c == "break"));
    ui.set_category_breakfast(g.categories.iter().any(|c| c == "breakfast"));
    ui.set_category_lunch(g.categories.iter().any(|c| c == "lunch"));
    ui.set_category_dinner(g.categories.iter().any(|c| c == "dinner"));
    ui.set_category_exercise(g.categories.iter().any(|c| c == "exercise"));
    ui.set_model_paths_json(
        serde_json::to_string(&g.model_paths)
            .expect("paths serialize")
            .into(),
    );
    ui.set_path_model_names(strings(
        TEXT_MODELS
            .iter()
            .chain(VOICE_MODELS.iter().filter(|id| **id != "system"))
            .map(|id| (*id).to_string()),
    ));
    show_path(ui);
    ui.set_runtime_seconds(g.runtime_wait_seconds.into());
    ui.set_character_description(g.character_description.as_str().into());
    ui.set_custom_instruction(g.cosy_instruction.as_str().into());
    ui.set_delivery_json(g.delivery_catalog.to_string().into());
    refresh_deliveries(ui);
    let index = match g.cosy_style.as_str() {
        "auto" => 0,
        "custom" => 1,
        _ => g
            .delivery_catalog
            .as_array()
            .and_then(|a| a.iter().position(|v| v["id"] == g.cosy_style))
            .map(|i| i + 2)
            .unwrap_or(0),
    };
    ui.set_delivery_index(index as i32);
    let ids = ui.get_voice_profile_ids();
    ui.set_voice_profile_index(
        (0..ids.row_count())
            .find(|i| ids.row_data(*i).is_some_and(|v| v == g.voice_profile))
            .unwrap_or(0) as i32,
    );
}
pub fn collect(ui: &AppWindow, p: &mut Preferences) -> Result<(), String> {
    let g = &mut p.generation;
    g.device = ["auto", "cpu", "cuda"]
        .get(ui.get_compute_device() as usize)
        .ok_or("invalid compute device")?
        .to_string();
    let text = |i: i32| {
        TEXT_MODELS
            .get(i as usize)
            .map(|s| s.to_string())
            .ok_or("invalid text model")
    };
    g.text_model = text(ui.get_text_model_index())?;
    g.runtime_text_model = text(ui.get_runtime_model_index())?;
    g.tts_model = VOICE_MODELS
        .get(ui.get_voice_model_index() as usize)
        .ok_or("invalid voice model")?
        .to_string();
    g.text_enabled = ui.get_text_enabled();
    g.runtime = ui.get_runtime_generation();
    g.auto_daily = ui.get_auto_daily();
    g.daily_auto_keep = ui.get_auto_keep();
    g.ac_only = ui.get_generation_ac_only();
    g.swear = ui.get_swear();
    g.batch_count = u16::try_from(ui.get_batch_count()).map_err(|e| e.to_string())?;
    g.categories = [
        ("break", ui.get_category_break()),
        ("breakfast", ui.get_category_breakfast()),
        ("lunch", ui.get_category_lunch()),
        ("dinner", ui.get_category_dinner()),
        ("exercise", ui.get_category_exercise()),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(id, _)| id.into())
    .collect();
    g.model_paths =
        serde_json::from_str(ui.get_model_paths_json().as_str()).map_err(|e| e.to_string())?;
    g.runtime_wait_seconds = u16::try_from(ui.get_runtime_seconds()).map_err(|e| e.to_string())?;
    g.character_description = ui.get_character_description().to_string();
    let ids = ui.get_voice_profile_ids();
    if ids.row_count() > 0 {
        g.voice_profile = ids
            .row_data(ui.get_voice_profile_index() as usize)
            .ok_or("invalid voice profile")?
            .to_string();
    }
    g.delivery_catalog =
        serde_json::from_str(ui.get_delivery_json().as_str()).map_err(|e| e.to_string())?;
    g.cosy_style = match ui.get_delivery_index() {
        0 => "auto".into(),
        1 => "custom".into(),
        i => g
            .delivery_catalog
            .as_array()
            .and_then(|a| a.get(i as usize - 2))
            .and_then(|v| v["id"].as_str())
            .ok_or("invalid delivery")?
            .into(),
    };
    g.cosy_instruction = ui.get_custom_instruction().to_string();
    g.validate()
}
fn refresh_deliveries(ui: &AppWindow) {
    if let Ok(Value::Array(items)) = serde_json::from_str(ui.get_delivery_json().as_str()) {
        ui.set_delivery_names(strings(
            [
                "Авто / Auto".to_string(),
                "Своя инструкция / Custom".to_string(),
            ]
            .into_iter()
            .chain(
                items
                    .iter()
                    .filter_map(|v| v["name"].as_str().map(str::to_owned)),
            ),
        ));
    }
}
pub(crate) fn render(ui: &AppWindow, json: &str) -> Result<(), String> {
    let mut view: View = serde_json::from_str(json).map_err(|e| e.to_string())?;
    view.english = ui.get_english();
    let en = view.english;
    if view.task_kind == "preset" || view.task_kind == "preset_import" {
        ui.set_character_feedback(local_status(&view.status, en).into());
    }
    let support = strings(view.support_texts.clone());
    if ui.get_support_texts().iter().collect::<Vec<_>>() != support.iter().collect::<Vec<_>>() {
        ui.set_support_texts(support);
    }
    ui.set_support_index(view.support_index as i32);
    ui.set_generation_running(view.running);
    ui.set_library_status(crate::display_path::text(&local_status(&view.status, en)).into());
    ui.set_task_title(
        match view.task_kind.as_str() {
            "install" => {
                if en {
                    "Model setup"
                } else {
                    "Подготовка моделей"
                }
            }
            "setup" => {
                if en {
                    "Setup required"
                } else {
                    "Требуется подготовка"
                }
            }
            "daily" => {
                if en {
                    "Preparing today"
                } else {
                    "Подготовка дня"
                }
            }
            "support" => {
                if en {
                    "Daily encouragement"
                } else {
                    "Фразы поддержки"
                }
            }
            "batch" => {
                if en {
                    "New phrases"
                } else {
                    "Создание фраз"
                }
            }
            "speech" => {
                if en {
                    "Speech generation"
                } else {
                    "Озвучка фраз"
                }
            }
            "runtime" => {
                if en {
                    "Reminder phrase"
                } else {
                    "Фраза напоминания"
                }
            }
            "export" => {
                if en {
                    "Export"
                } else {
                    "Экспорт"
                }
            }
            "preset" | "preset_import" => {
                if en {
                    "Character presets"
                } else {
                    "Пресеты персонажа"
                }
            }
            _ => {
                if en {
                    "Tasks"
                } else {
                    "Задачи"
                }
            }
        }
        .into(),
    );
    ui.set_task_progress(view.task_progress.unwrap_or(-1.) as f32);
    ui.set_task_error(view.task_error);
    ui.set_task_details(
        crate::display_path::text(&format!("{}\n{}", view.status, view.task_details)).into(),
    );
    let page_audio = view.audio_available.len();
    ui.set_library_current_count(view.today_total as i32);
    ui.set_library_history_count(view.all_total.saturating_sub(view.today_total) as i32);
    ui.set_library_counts(
        if en {
            format!(
                "All: {} · Today: {} · Playable on page: {}/{}",
                view.all_total,
                view.today_total,
                page_audio,
                view.phrases.len()
            )
        } else {
            format!(
                "Всего: {} · Сегодня: {} · Можно прослушать на странице: {}/{}",
                view.all_total,
                view.today_total,
                page_audio,
                view.phrases.len()
            )
        }
        .into(),
    );
    ui.set_library_summary(
        format!(
            "{} · {}: {} · {}: {}",
            if en {
                "Working copy"
            } else {
                "Рабочая копия"
            },
            if en { "phrases" } else { "фраз" },
            view.total,
            if en {
                "imported"
            } else {
                "перенесено"
            },
            view.imported
        )
        .into(),
    );
    ui.set_library_page_index(view.page as i32);
    ui.set_library_has_next((view.page + 1) * 50 < view.total);
    ui.set_library_page_caption(
        format!(
            "{}–{} / {}",
            (view.page * 50 + 1).min(view.total),
            (view.page * 50 + view.phrases.len()).min(view.total),
            view.total
        )
        .into(),
    );
    ui.set_archive_page_index(view.archived_page as i32);
    ui.set_archive_has_next((view.archived_page + 1) * 10 < view.archived_total);
    ui.set_archive_total(view.archived_total as i32);
    let zone = &view.time_zone;
    ui.set_archived_notices(ModelRc::new(VecModel::from(
        view.archived_pending
            .iter()
            .map(|n| crate::PendingItem {
                identifier: n.key.as_str().into(),
                text: n.text.as_str().into(),
                caption: kavychka_platform::time::local_in(n.created, zone)
                    .map(|date| {
                        date.format(if en {
                            "%Y-%m-%d %H:%M"
                        } else {
                            "%d.%m.%Y %H:%M"
                        })
                        .to_string()
                    })
                    .unwrap_or_else(|_| n.created.to_rfc3339())
                    .into(),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_library_day(view.day.as_str().into());
    let list = view
        .phrases
        .iter()
        .map(|r| LibraryItem {
            selected: false,
            state: phrase_status(r, &view).0.into(),
            state_hint: phrase_status(r, &view).1.into(),
            identifier: r.id.as_str().into(),
            text: r.text.as_str().into(),
            caption: format!(
                "{} · {} · {}{}{} · {}{}",
                r.day
                    .as_deref()
                    .map(|d| if d == view.day {
                        if en { "Today" } else { "Сегодня" }
                    } else {
                        d
                    })
                    .unwrap_or("—"),
                match r.category.as_str() {
                    "break" =>
                        if en {
                            "Break"
                        } else {
                            "Перерыв"
                        },
                    "breakfast" =>
                        if en {
                            "Breakfast"
                        } else {
                            "Завтрак"
                        },
                    "lunch" =>
                        if en {
                            "Lunch"
                        } else {
                            "Обед"
                        },
                    "dinner" =>
                        if en {
                            "Dinner"
                        } else {
                            "Ужин"
                        },
                    _ =>
                        if en {
                            "Stretch"
                        } else {
                            "Разминка"
                        },
                },
                view.models
                    .iter()
                    .find(|m| m.id == r.model)
                    .map(|m| m.name.split(" · ").next().unwrap_or(m.name.as_str()))
                    .unwrap_or(&r.model),
                if r.used_at.is_some() {
                    if en {
                        " · used"
                    } else {
                        " · использована"
                    }
                } else {
                    ""
                },
                if r.rejected {
                    if en {
                        " · excluded"
                    } else {
                        " · исключена"
                    }
                } else if r.approved {
                    if en {
                        " · kept"
                    } else {
                        " · оставлена"
                    }
                } else {
                    if en {
                        " · review"
                    } else {
                        " · на отбор"
                    }
                },
                if r.model == "system" {
                    if en {
                        "System voice"
                    } else {
                        "Системный голос"
                    }
                } else if view.audio_available.contains(&r.id) {
                    if en {
                        "Audio ready"
                    } else {
                        "Запись готова"
                    }
                } else {
                    if en {
                        "No recording"
                    } else {
                        "Без записи"
                    }
                },
                if r.voice_style.is_empty() {
                    String::new()
                } else {
                    let deliveries: Value =
                        serde_json::from_str(ui.get_delivery_json().as_str()).unwrap_or_default();
                    let name = deliveries
                        .as_array()
                        .and_then(|a| a.iter().find(|v| v["id"] == r.voice_style))
                        .and_then(|v| v["name"].as_str())
                        .unwrap_or(&r.voice_style);
                    format!(" · {name}")
                }
            )
            .into(),
            kept: r.approved,
            excluded: r.rejected,
            has_audio: view.audio_available.contains(&r.id),
        })
        .collect::<Vec<_>>();
    let list = crate::library_actions::decorate(ui, list);
    let current = ui.get_library_items();
    if let Some(model) = current.as_any().downcast_ref::<VecModel<LibraryItem>>() {
        if model.row_count() == list.len()
            && list.iter().enumerate().all(|(i, row)| {
                model
                    .row_data(i)
                    .is_some_and(|old| old.identifier == row.identifier)
            })
        {
            for (i, row) in list.into_iter().enumerate() {
                if model.row_data(i).as_ref() != Some(&row) {
                    model.set_row_data(i, row);
                }
            }
        } else {
            model.set_vec(list);
        }
    } else {
        ui.set_library_items(ModelRc::new(VecModel::from(list)));
    }
    let mut text_readiness = vec![];
    let mut voice_readiness = vec![];
    let mut text_names = vec![];
    let mut text_notes = vec![];
    let mut voice_names = vec![];
    let mut voice_notes = vec![];
    for (ids, names, notes, readiness) in [
        (
            TEXT_MODELS,
            &mut text_names,
            &mut text_notes,
            &mut text_readiness,
        ),
        (
            VOICE_MODELS,
            &mut voice_names,
            &mut voice_notes,
            &mut voice_readiness,
        ),
    ] {
        for id in ids {
            let model = view.models.iter().find(|m| m.id == *id);
            let name = model
                .map(|m| m.name.clone())
                .unwrap_or_else(|| id.to_string());
            let installed = model.is_some_and(|m| m.installed && m.environment);
            readiness.push(
                match model {
                    Some(m) if m.installed && m.environment => {
                        if en {
                            "Ready to use"
                        } else {
                            "Можно использовать"
                        }
                    }
                    Some(m) if m.installed => {
                        if en {
                            "Model files found. Set up the required components."
                        } else {
                            "Файлы модели найдены. Требуется подготовить компоненты запуска."
                        }
                    }
                    Some(m) if m.environment => {
                        if en {
                            "Components ready. Download the model files."
                        } else {
                            "Компоненты готовы. Требуется загрузить файлы модели."
                        }
                    }
                    _ => {
                        if en {
                            "Set up this model before generating new phrases."
                        } else {
                            "Для новых фраз подготовь эту модель."
                        }
                    }
                }
                .to_string(),
            );
            names.push(format!(
                "{}{}",
                name,
                if installed {
                    if en {
                        " · installed"
                    } else {
                        " · установлена"
                    }
                } else {
                    if en {
                        " · setup needed"
                    } else {
                        " · нужна подготовка"
                    }
                }
            ));
            notes.push(
                model
                    .map(|m| {
                        let estimate=if m.id=="system"{if en{"Basic mode; no local model memory".into()}else{"Базовый режим; память локальной модели не нужна".into()}}else{
                            let reserve=if m.id.starts_with("tts-")||m.id=="text-1.7b"{4.0}else{2.0};
                            let ram=m.bytes as f64/1e9+reserve;
                            if en{format!("CPU RAM planning estimate: {ram:.1} GB + OS. CUDA VRAM depends on engine/offload; not a measured minimum.")}else{format!("Ориентир RAM на CPU: {ram:.1} ГБ + ОС. VRAM CUDA зависит от движка/выгрузки; это не измеренный минимум.")}
                        };
                        format!(
                            "{:.2} GB · {}{}\n{estimate}",
                            m.bytes as f64 / 1e9,
                            crate::display_path::display(&m.path),
                            if m.installed && !m.environment {
                                if en {" · components need setup"} else {" · нужны компоненты запуска"}
                            } else {
                                ""
                            }
                        )
                    })
                    .unwrap_or_default(),
            );
        }
    }
    ui.set_text_catalog(strings(text_names));
    ui.set_text_readiness_list(strings(text_readiness));
    ui.set_voice_readiness_list(strings(voice_readiness));
    ui.set_voice_catalog(strings(voice_names));
    ui.set_text_notes(strings(text_notes));
    ui.set_voice_notes(strings(voice_notes));
    let selected_profile = if ui.get_voice_profile_ids().row_count() == 0 {
        serde_json::from_str::<Preferences>(ui.get_loaded_prefs().as_str())
            .map(|p| p.generation.voice_profile)
            .unwrap_or_default()
    } else {
        ui.get_voice_profile_ids()
            .row_data(ui.get_voice_profile_index().max(0) as usize)
            .map(|id| id.to_string())
            .unwrap_or_default()
    };
    ui.set_voice_profile_ids(strings(view.profiles.iter().map(|(id, _)| id.clone())));
    ui.set_voice_profile_names(strings(view.profiles.iter().map(|(id, name)| {
        if id.is_empty() {
            if en {
                "Kavychka · default voice".into()
            } else {
                "Кавычка · обычный голос".into()
            }
        } else {
            name.clone()
        }
    })));
    ui.set_voice_profile_index(
        ui.get_voice_profile_ids()
            .iter()
            .position(|id| id == selected_profile.as_str())
            .unwrap_or(0) as i32,
    );
    crate::presets_ui::render(ui, &view.presets);
    if !ui.get_dirty()
        && let Ok(p) = serde_json::from_str::<Preferences>(ui.get_loaded_prefs().as_str())
    {
        apply(ui, &p);
    }
    Ok(())
}
pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    ui.on_retranslate(move || {
        if let Some(ui) = weak.upgrade() {
            let json = ui.get_library_json();
            if !json.is_empty() {
                let _ = render(&ui, json.as_str());
            }
            crate::presets_ui::retranslate(&ui);
            crate::shell_settings::retranslate(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_path_selected(move || {
        if let Some(ui) = weak.upgrade() {
            show_path(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_path_edited(move || {
        if let Some(ui) = weak.upgrade() {
            let mut paths: std::collections::BTreeMap<String, String> =
                serde_json::from_str(ui.get_model_paths_json().as_str()).unwrap_or_default();
            if let Some(id) = path_id(ui.get_path_model_index()) {
                let value = ui.get_path_model_value().to_string();
                if value.trim().is_empty() {
                    paths.remove(id);
                } else {
                    paths.insert(id.into(), value);
                }
                ui.set_model_paths_json(
                    serde_json::to_string(&paths)
                        .expect("paths serialize")
                        .into(),
                );
            }
        }
    });

    crate::presets_ui::wire(ui, backend);
    crate::library_actions::wire(ui, backend);
    let weak = ui.as_weak();
    ui.on_library_update(move |json| {
        if let Some(ui) = weak.upgrade() {
            ui.set_library_json(json.clone());
            if let Err(e) = render(&ui, json.as_str()) {
                ui.set_library_status(e.into());
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_library_filter(move || {
        if let Some(ui) = weak.upgrade() {
            crate::library_actions::clear(&ui);
            ui.set_library_scroll_y(0.);
            ui.set_library_controls(false);
            if let Err(e) = sender.try_send(Command::LibraryPage(ui.get_library_today(), 0)) {
                ui.set_library_status(e.to_string().into());
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_library_page(move |direction| {
        if let Some(ui) = weak.upgrade() {
            let page = (ui.get_library_page_index() + direction).max(0) as usize;
            ui.set_library_scroll_y(0.);
            if let Err(e) = sender.try_send(Command::LibraryPage(ui.get_library_today(), page)) {
                ui.set_library_status(e.to_string().into());
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_archive_page(move |direction| {
        if let Some(ui) = weak.upgrade() {
            let page = (ui.get_archive_page_index() + direction).max(0) as usize;
            if let Err(e) = sender.try_send(Command::ArchivePage(page)) {
                ui.set_library_status(e.to_string().into());
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_confirm_archived(move |key| {
        if let Err(e) = sender.try_send(Command::ConfirmArchived(key.to_string()))
            && let Some(ui) = weak.upgrade()
        {
            ui.set_library_status(e.to_string().into());
        }
    });
    let dispatch = {
        let weak = ui.as_weak();
        let sender = backend.sender.clone();
        move |command, needs_saved| {
            if let Some(ui) = weak.upgrade() {
                if needs_saved && ui.get_dirty() {
                    ui.set_library_status("Сначала сохрани настройки / Save settings first".into());
                    return;
                }
                if let Err(e) = sender.try_send(command) {
                    ui.set_library_status(e.to_string().into());
                }
            }
        }
    };
    let call = dispatch.clone();
    ui.on_prepare_day(move || call(Command::PrepareDay, true));
    let call = dispatch.clone();
    ui.on_generate_batch(move || call(Command::GenerateBatch, true));
    let call = dispatch.clone();
    ui.on_generate_on_battery(move || call(Command::GenerateBatchOnBattery, true));
    let call = dispatch.clone();
    ui.on_regenerate_speech(move |id| {
        call(
            Command::RegenerateSpeech(if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }),
            true,
        )
    });
    let call = dispatch.clone();
    ui.on_select_phrase(move |id, keep| call(Command::SelectPhrase(id.to_string(), keep), false));
    let call = dispatch.clone();
    ui.on_play_phrase(move |id| call(Command::PlayPhrase(id.to_string()), false));
    let call = dispatch.clone();
    ui.on_export_library(move || call(Command::ExportLibrary, false));
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_install_models(move || {
        if let Some(ui) = weak.upgrade() {
            let result = (|| -> Result<(), String> {
                let base: Preferences = serde_json::from_str(ui.get_loaded_prefs().as_str())
                    .map_err(|e| e.to_string())?;
                let mut desired = base.clone();
                collect_setup(&ui, &mut desired)?;
                sender
                    .try_send(Command::PrepareModels(
                        Box::new(desired.generation),
                        Box::new(base.generation),
                    ))
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                ui.set_library_status(error.into());
            }
        }
    });
    let call = dispatch.clone();
    ui.on_repair_environment(move || call(Command::RepairEnvironment, true));
    let call = dispatch.clone();
    ui.on_refresh_library(move || call(Command::RefreshLibrary, false));
    let call = dispatch.clone();
    ui.on_clear_recordings(move || call(Command::ClearRecordings, false));
    let call = dispatch.clone();
    ui.on_hide_phrase(move |id| call(Command::HidePhrase(id.to_string()), false));
    let cancellation = backend.cancellation.clone();
    ui.on_cancel_generation(move || cancellation.store(true, std::sync::atomic::Ordering::Release));
    let weak = ui.as_weak();
    ui.on_add_delivery(move || {
        if let Some(ui) = weak.upgrade() {
            let result = (|| -> Result<(), String> {
                let mut items: Vec<Value> = serde_json::from_str(ui.get_delivery_json().as_str())
                    .map_err(|e| e.to_string())?;
                let name = ui.get_delivery_name().trim().to_owned();
                let instruction = ui.get_delivery_instruction().trim().to_owned();
                if name.is_empty() || instruction.is_empty() {
                    return Err("Заполни название и инструкцию / Enter name and instruction".into());
                }
                let id = format!("user_{}", chrono::Utc::now().timestamp_millis());
                items.push(json!({"id":id,"name":name,"instruction":instruction}));
                let p: Preferences = serde_json::from_str(ui.get_loaded_prefs().as_str())
                    .map_err(|e| e.to_string())?;
                let mut g = p.generation;
                g.delivery_catalog = json!(items);
                g.validate()?;
                ui.set_delivery_json(g.delivery_catalog.to_string().into());
                refresh_deliveries(&ui);
                ui.set_delivery_index(items.len() as i32 + 1);
                ui.invoke_edited();
                Ok(())
            })();
            if let Err(e) = result {
                ui.set_status(e.into());
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_remove_delivery(move || {
        if let Some(ui) = weak.upgrade()
            && let Ok(mut items) =
                serde_json::from_str::<Vec<Value>>(ui.get_delivery_json().as_str())
        {
            let index = ui.get_delivery_index();
            if index >= 2 && items.len() > 1 {
                items.remove(index as usize - 2);
                ui.set_delivery_json(json!(items).to_string().into());
                refresh_deliveries(&ui);
                ui.set_delivery_index(0);
                ui.invoke_edited();
            } else {
                ui.set_status("Оставь хотя бы одну подачу / Keep at least one delivery".into());
            }
        }
    });
}

fn path_id(index: i32) -> Option<&'static str> {
    usize::try_from(index).ok().and_then(|index| {
        TEXT_MODELS
            .iter()
            .chain(VOICE_MODELS.iter().filter(|id| **id != "system"))
            .nth(index)
            .copied()
    })
}
fn show_path(ui: &AppWindow) {
    let paths: std::collections::BTreeMap<String, String> =
        serde_json::from_str(ui.get_model_paths_json().as_str()).unwrap_or_default();
    ui.set_path_model_value(
        path_id(ui.get_path_model_index())
            .and_then(|id| paths.get(id))
            .map(|s| crate::display_path::readable(s))
            .unwrap_or_default()
            .into(),
    );
}

// The setup button saves only controls on this page, never another page's draft.
fn collect_setup(ui: &AppWindow, p: &mut Preferences) -> Result<(), String> {
    let g = &mut p.generation;
    g.device = ["auto", "cpu", "cuda"]
        .get(ui.get_compute_device() as usize)
        .ok_or("invalid compute device")?
        .to_string();
    g.text_model = TEXT_MODELS
        .get(ui.get_text_model_index() as usize)
        .ok_or("invalid text model")?
        .to_string();
    g.runtime_text_model = TEXT_MODELS
        .get(ui.get_runtime_model_index() as usize)
        .ok_or("invalid runtime model")?
        .to_string();
    g.tts_model = VOICE_MODELS
        .get(ui.get_voice_model_index() as usize)
        .ok_or("invalid voice model")?
        .to_string();
    let voices = ui.get_voice_profile_ids();
    if voices.row_count() > 0 {
        g.voice_profile = voices
            .row_data(ui.get_voice_profile_index() as usize)
            .ok_or("invalid voice profile")?
            .to_string();
    }
    g.text_enabled = ui.get_text_enabled();
    g.runtime = ui.get_runtime_generation();
    g.auto_daily = ui.get_auto_daily();
    g.daily_auto_keep = ui.get_auto_keep();
    g.ac_only = ui.get_generation_ac_only();
    g.model_paths =
        serde_json::from_str(ui.get_model_paths_json().as_str()).map_err(|e| e.to_string())?;
    g.runtime_wait_seconds = u16::try_from(ui.get_runtime_seconds()).map_err(|e| e.to_string())?;
    g.validate()
}

pub fn setup_saved(
    ui: &AppWindow,
    desired: &kavychka_core::generation::Generation,
    base: &kavychka_core::generation::Generation,
) {
    // Rebase only fields this button committed. Other pages retain their original
    // baseline, so a later Save still merges concurrent changes correctly.
    if let Ok(mut loaded) = serde_json::from_str::<Preferences>(ui.get_loaded_prefs().as_str()) {
        let mut baseline = loaded.clone();
        let mut changed = loaded.clone();
        baseline.generation = base.clone();
        changed.generation = desired.clone();
        if let Ok(merged) = kavychka_core::preferences::merge(&loaded, &baseline, &changed) {
            loaded.generation = merged.generation;
            if let Ok(json) = serde_json::to_string(&loaded) {
                ui.set_loaded_prefs(json.into());
                ui.set_dirty(crate::collect(ui).map_or(true, |draft| {
                    serde_json::to_value(draft).ok() != serde_json::to_value(loaded).ok()
                }));
            }
        }
    }
}
