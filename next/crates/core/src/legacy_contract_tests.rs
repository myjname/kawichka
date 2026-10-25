//! Executable field contract. Synthetic values deliberately differ from defaults.
use super::*;
use crate::{engine::Clock, legacy_active, legacy_state, preferences::AudioMode};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::BTreeSet;

fn contract() -> Value {
    serde_json::from_str(include_str!("../../../contracts/p1-fields.json")).unwrap()
}

#[test]
fn every_allowed_field_has_an_explicit_disposition_and_active_values_survive() {
    let c = contract();
    assert_eq!(c["contract"], 2);
    for (section, allowed) in [("desktop", DESKTOP_FIELDS), ("backend", INFERENCE_FIELDS)] {
        let rows = c[section].as_array().unwrap();
        let keys: BTreeSet<_> = rows.iter().map(|r| r[0].as_str().unwrap()).collect();
        assert_eq!(keys.len(), rows.len(), "duplicate {section} field");
        assert_eq!(
            keys,
            allowed.iter().copied().collect(),
            "{section} coverage"
        );
        for r in rows {
            assert_eq!(r.as_array().unwrap().len(), 4);
            assert!(
                [
                    "active", "state", "source", "archive", "policy", "derived", "shared"
                ]
                .contains(&r[1].as_str().unwrap())
            );
            assert!(!r[2].as_str().unwrap().is_empty());
        }
    }
    let desktop = c["desktop"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_str().unwrap().into(), r[3].clone()))
        .collect();
    let backend = c["backend"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_str().unwrap().into(), r[3].clone()))
        .collect();
    let snapshot = reconcile(&desktop, &backend).unwrap();
    let prefs = legacy_active::preferences(&snapshot).unwrap();
    let active = serde_json::to_value(&prefs).unwrap();
    let utc: DateTime<Utc> = "2026-10-23T12:02:00Z".parse().unwrap();
    let state = legacy_state::convert(
        &snapshot,
        &Value::Null,
        &prefs,
        Clock {
            utc,
            mono: 0,
            date: utc.date_naive(),
        },
        |d: NaiveDate, m| {
            Ok(d.and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)
                .unwrap()
                .and_utc())
        },
    )
    .unwrap();
    let saved = serde_json::to_value(&state).unwrap();
    for section in ["desktop", "backend"] {
        for r in c[section].as_array().unwrap() {
            let key = r[0].as_str().unwrap();
            match r[1].as_str().unwrap() {
                "active" => assert_eq!(
                    active.pointer(r[2].as_str().unwrap()),
                    Some(&r[3]),
                    "{section}.{key}"
                ),
                "state" => assert_eq!(
                    saved.pointer(r[2].as_str().unwrap()),
                    Some(&r[3]),
                    "{section}.{key}"
                ),
                "shared" => {
                    assert!(!snapshot.inference.contains_key(key));
                    assert_eq!(snapshot.desktop.get(r[2].as_str().unwrap()), Some(&r[3]));
                }
                _ => {
                    let map = if section == "desktop" {
                        &snapshot.desktop
                    } else {
                        &snapshot.inference
                    };
                    assert_eq!(map.get(key), Some(&r[3]), "archive {section}.{key}");
                }
            }
        }
    }
    assert_eq!(prefs.reminders.audio, AudioMode::Chime);
    assert!(!state.pause_forever);
    assert!(state.handled.contains_key("2026-10-23:600:break"));
    assert!(state.handled.contains_key("2026-10-23:780:lunch"));
    assert!(state.handled.contains_key("2026-10-23:540:break"));
    assert_eq!(
        state.exercise.unwrap().due.to_rfc3339(),
        "2026-10-23T13:30:00+00:00"
    );
    assert_eq!(
        state.breakfast_times[&utc.date_naive()].to_rfc3339(),
        "2026-10-23T12:00:00+00:00"
    );
}

#[test]
fn shared_precedence_covers_each_pair_flag_override_and_missing_owner() {
    for &(old, key, _) in SHARED {
        let daily = matches!(key, "auto_daily" | "daily_auto_keep");
        let flag = if daily {
            "BackendDailySettingsMigrated"
        } else {
            "BackendSettingsMigrated"
        };
        let boolean = matches!(key, "swear" | "runtime" | "auto_daily" | "daily_auto_keep");
        let (a, b) = if boolean {
            (json!(true), json!(false))
        } else {
            (json!("desktop-choice"), json!("database-choice"))
        };
        for migrated in [false, true] {
            for manual in [false, true] {
                let mut d = Map::new();
                d.insert(old.into(), a.clone());
                d.insert(flag.into(), json!(migrated));
                // The independent flag must not affect this pair.
                d.insert(
                    if daily {
                        "BackendSettingsMigrated"
                    } else {
                        "BackendDailySettingsMigrated"
                    }
                    .into(),
                    json!(!migrated),
                );
                if manual {
                    d.insert("BackendSettingsOverrides".into(), json!([key]));
                }
                let db = BTreeMap::from([(key.into(), b.clone())]);
                assert_eq!(
                    reconcile(&d, &db).unwrap().desktop[old],
                    if migrated || manual {
                        a.clone()
                    } else {
                        b.clone()
                    },
                    "{key}/{migrated}/{manual}"
                );
                d.remove(old);
                assert_eq!(reconcile(&d, &db).unwrap().desktop[old], b);
                d.insert(old.into(), a.clone());
                assert_eq!(reconcile(&d, &BTreeMap::new()).unwrap().desktop[old], a);
            }
        }
    }
}

#[test]
fn bounded_schema_versions_reject_malformed_markers_without_reset() {
    for v in [
        json!(-1),
        json!("2"),
        json!(null),
        json!(true),
        json!(1.5),
        json!(3),
    ] {
        assert!(
            reconcile(
                json!({"SchemaVersion":v}).as_object().unwrap(),
                &BTreeMap::new()
            )
            .is_err()
        );
    }
    for v in [None, Some(0), Some(1), Some(2)] {
        let mut d = Map::new();
        if let Some(v) = v {
            d.insert("SchemaVersion".into(), json!(v));
        }
        assert!(reconcile(&d, &BTreeMap::new()).is_ok());
    }
}
