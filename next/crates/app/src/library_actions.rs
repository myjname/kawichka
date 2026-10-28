//! Ephemeral library selections, shared across pages but never across categories/days.
use crate::{
    AppWindow, LibraryItem,
    service::{Backend, Command},
};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub group: String,
    pub day: String,
    pub ids: BTreeSet<String>,
}
impl Selection {
    pub fn validate(&self, day: &str, action: &str) -> Result<(), String> {
        if self.day != day || !["current", "history", "support"].contains(&self.group.as_str()) {
            return Err(
                "Список изменился. Выбери фразы заново / List changed; select again".into(),
            );
        }
        if !["all", "delete", "keep", "exclude", "speech", "export"].contains(&action)
            || self.group != "current" && !["all", "delete"].contains(&action)
            || action != "all" && self.ids.is_empty()
        {
            return Err(
                "Действие недоступно для этого списка / Action unavailable for this list".into(),
            );
        }
        Ok(())
    }
}
fn group(ui: &AppWindow) -> &str {
    if ui.get_library_support() {
        "support"
    } else if ui.get_library_today() {
        "current"
    } else {
        "history"
    }
}
pub fn selection(ui: &AppWindow) -> Selection {
    let mut s: Selection =
        serde_json::from_str(ui.get_library_selection().as_str()).unwrap_or_default();
    if s.group != group(ui) || s.day != ui.get_library_day().as_str() {
        s = Selection {
            group: group(ui).into(),
            day: ui.get_library_day().to_string(),
            ids: Default::default(),
        };
    }
    s
}
fn redraw(ui: &AppWindow) {
    if let Err(e) = crate::models_ui::render(ui, ui.get_library_json().as_str()) {
        ui.set_library_status(e.into());
    }
}
fn set(ui: &AppWindow, s: &Selection) {
    ui.set_library_selection(
        serde_json::to_string(s)
            .expect("selection serializes")
            .into(),
    );
    redraw(ui);
}
pub fn clear(ui: &AppWindow) {
    set(
        ui,
        &Selection {
            ids: Default::default(),
            ..selection(ui)
        },
    );
}
pub fn clear_if_unchanged(ui: &AppWindow, original: &Selection) {
    if selection(ui) == *original {
        clear(ui);
    }
}
pub fn accept_all(ui: &AppWindow, original: &Selection, ids: Vec<String>) {
    if selection(ui) == *original {
        set(
            ui,
            &Selection {
                ids: ids.into_iter().collect(),
                ..original.clone()
            },
        );
    }
}
pub fn decorate(ui: &AppWindow, items: Vec<LibraryItem>) -> Vec<LibraryItem> {
    let s = selection(ui);
    ui.set_library_selected_count(s.ids.len() as i32);
    let total = if ui.get_library_support() {
        ui.get_support_texts().row_count() as i32
    } else if ui.get_library_today() {
        ui.get_library_current_count()
    } else {
        ui.get_library_history_count()
    };
    ui.set_library_all_selected(total > 0 && s.ids.len() == total as usize);
    ui.set_support_items(slint::ModelRc::new(slint::VecModel::from(
        ui.get_support_texts()
            .iter()
            .map(|text| LibraryItem {
                identifier: text.clone(),
                text: text.clone(),
                selected: s.group == "support" && s.ids.contains(text.as_str()),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    items
        .into_iter()
        .map(|mut row| {
            row.selected = s.group != "support" && s.ids.contains(row.identifier.as_str());
            row
        })
        .collect()
}
pub fn wire(ui: &AppWindow, backend: &Backend) {
    let weak = ui.as_weak();
    ui.on_toggle_library_item(move |id| {
        if let Some(ui) = weak.upgrade() {
            let mut s = selection(&ui);
            if !s.ids.remove(id.as_str()) {
                s.ids.insert(id.to_string());
            }
            set(&ui, &s);
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_select_all_library(move |checked| {
        if let Some(ui) = weak.upgrade() {
            if !checked {
                clear(&ui);
            } else if let Err(e) =
                sender.try_send(Command::LibrarySelection(selection(&ui), "all".into()))
            {
                ui.set_library_status(e.to_string().into());
            }
        }
    });
    let weak = ui.as_weak();
    let sender = backend.sender.clone();
    ui.on_library_action(move |action| {
        if let Some(ui) = weak.upgrade() {
            let s = selection(&ui);
            let result = (|| -> Result<(), String> {
                if action == "play" {
                    if s.group != "current" || s.ids.len() != 1 {
                        return Err(
                            "Выбери одну актуальную фразу / Select one current phrase".into()
                        );
                    }
                    sender
                        .try_send(Command::PlayPhrase(s.ids.iter().next().unwrap().clone()))
                        .map_err(|e| e.to_string())?;
                } else {
                    s.validate(ui.get_library_day().as_str(), action.as_str())?;
                    if ["speech", "export"].contains(&action.as_str()) && ui.get_dirty() {
                        return Err("Сначала сохрани настройки / Save settings first".into());
                    }
                    sender
                        .try_send(Command::LibrarySelection(s, action.to_string()))
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                ui.set_library_status(e.into());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_support_and_stale_days_cannot_change_or_voice_phrases() {
        for group in ["history", "support"] {
            let s = Selection {
                group: group.into(),
                day: "2026-10-28".into(),
                ids: BTreeSet::from(["id".into()]),
            };
            assert!(s.validate("2026-10-28", "delete").is_ok());
            for action in ["keep", "exclude", "speech", "export"] {
                assert!(s.validate("2026-10-28", action).is_err());
            }
            assert!(s.validate("2026-10-29", "delete").is_err());
        }
    }
}
