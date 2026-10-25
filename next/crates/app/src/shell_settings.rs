//! Draft-only audio selection. Enumeration runs off the UI thread; a missing
//! pinned endpoint remains selected and is never replaced with the default.
use crate::{
    AppWindow,
    service::{Backend, Command},
};
use kavychka_core::preferences::{AudioMode, Language};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

pub fn refresh(ui: &AppWindow) {
    if ui.get_audio_refreshing() {
        return;
    }
    ui.set_audio_refreshing(true);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = kavychka_platform::audio::devices();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_audio_refreshing(false);
            let english = ui.get_english();
            let pinned = ui.get_audio_device_id();
            let mut names: Vec<SharedString> = vec![
                if english {
                    "System default"
                } else {
                    "Как в системе"
                }
                .into(),
            ];
            let mut ids: Vec<SharedString> = vec!["".into()];
            match result {
                Ok(devices) => {
                    for device in devices {
                        names.push(device.name.into());
                        ids.push(device.id.into());
                    }
                }
                Err(error) => ui.set_audio_test_status(error.into()),
            }
            let index = ids.iter().position(|id| id == &pinned).unwrap_or_else(|| {
                names.push(
                    if english {
                        "Selected device unavailable"
                    } else {
                        "Выбранное устройство недоступно"
                    }
                    .into(),
                );
                ids.push(pinned.clone());
                ids.len() - 1
            });
            ui.set_audio_device_names(ModelRc::new(VecModel::from(names)));
            ui.set_audio_device_ids(ModelRc::new(VecModel::from(ids)));
            ui.set_audio_device_index(index as i32);
        });
    });
}

pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    ui.on_refresh_audio(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui);
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_test_audio(move |kind| {
        if let Some(ui) = weak.upgrade() {
            let command = Command::TestAudio {
                device: ui.get_audio_device_id().to_string(),
                language: if ui.get_speech_index() == 1 {
                    Language::En
                } else {
                    Language::Ru
                },
                mode: match kind {
                    0 => AudioMode::Chime,
                    1 => AudioMode::Voice,
                    _ => AudioMode::All,
                },
            };
            ui.set_audio_test_status(
                if ui.get_english() {
                    "Testing…"
                } else {
                    "Проверяем…"
                }
                .into(),
            );
            if let Err(e) = sender.try_send(command) {
                ui.set_audio_test_status(e.to_string().into());
            }
        }
    });
}

pub fn select_saved(ui: &AppWindow, id: &str) {
    ui.set_audio_device_id(id.into());
    let ids = ui.get_audio_device_ids();
    if let Some(index) = ids.iter().position(|item| item == id) {
        ui.set_audio_device_index(index as i32);
    } else if id.is_empty() {
        ui.set_audio_device_names(ModelRc::new(VecModel::from(vec![SharedString::from(
            if ui.get_english() {
                "System default"
            } else {
                "Как в системе"
            },
        )])));
        ui.set_audio_device_ids(ModelRc::new(VecModel::from(vec![SharedString::from("")])));
        ui.set_audio_device_index(0);
    } else {
        // Preserve even before enumeration completes.
        let names = vec![
            SharedString::from("System default / Как в системе"),
            SharedString::from("Selected device unavailable / Устройство недоступно"),
        ];
        ui.set_audio_device_names(ModelRc::from(Rc::new(VecModel::from(names))));
        ui.set_audio_device_ids(ModelRc::new(VecModel::from(vec![
            SharedString::from(""),
            SharedString::from(id),
        ])));
        ui.set_audio_device_index(1);
    }
}
