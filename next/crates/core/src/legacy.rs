//! Reconcile the two legacy owners without retaining unknown credential fields.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid legacy settings field: {0}")]
    Invalid(String),
    #[error("unsupported desktop schema version")]
    FutureSchema,
}

/// One Rust-owned snapshot. Shared choices live only in `desktop`; inference
/// contains advanced choices, not a second copy of selected models or languages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingsSnapshot {
    pub format: u32,
    pub desktop: BTreeMap<String, Value>,
    pub inference: BTreeMap<String, Value>,
    pub ignored_desktop_fields: Vec<String>,
    pub ignored_inference_fields: Vec<String>,
}

const DESKTOP_FIELDS: &[&str] = &[
    "SchemaVersion",
    "SetupComplete",
    "PersonalizationMigrated",
    "BackendPersonalizationMigrated",
    "BackendSettingsMigrated",
    "BackendDailySettingsMigrated",
    "BackendSettingsOverrides",
    "LastNotifiedUpdate",
    "UiLanguage",
    "SpeechLanguage",
    "ThemeMode",
    "AddressForm",
    "UserName",
    "TimeZoneId",
    "WorkDays",
    "WorkStart",
    "WorkEnd",
    "BreakEvery",
    "BreakfastTime",
    "LunchTime",
    "DinnerTime",
    "MealsEnabled",
    "ExerciseAfter",
    "IdleMinutes",
    "AllowedClicks",
    "RepeatMinutes",
    "BreakRepeats",
    "ExerciseRepeats",
    "MealRepeats",
    "TrustConfirmation",
    "PauseUntilUtc",
    "FullscreenVisual",
    "FullscreenAudio",
    "DeferGenerationFullscreen",
    "PetX",
    "PetY",
    "PetSize",
    "VisualFile",
    "VoiceProfile",
    "CharacterFile",
    "PetVisible",
    "AutomaticUpdates",
    "RunAtLogin",
    "AutomaticPreparation",
    "AutoKeepDaily",
    "TextGeneration",
    "RuntimeGeneration",
    "TextModel",
    "RuntimeTextModel",
    "VoiceModel",
    "HandledEvents",
    "VoiceProvider",
    "Swear",
    "Voice",
    "NotificationSound",
    "Paused",
    "ExerciseEnabled",
    "LastMeal",
    "LastBreak",
    "ExerciseUtc",
    "History",
];
pub const INFERENCE_FIELDS: &[&str] = &[
    "tts_model",
    "runtime",
    "batch_count",
    "categories",
    "auto_daily",
    "daily_auto_keep",
    "swear",
    "device",
    "model_paths",
    "character_description",
    "text_model",
    "runtime_text_model",
    "cosy_style",
    "cosy_instruction",
    "delivery_catalog",
    "speech_language",
    "voice_profile",
    "reminder_followup",
    "address_form",
    "user_name",
];

const SHARED: &[(&str, &str, &str)] = &[
    ("TextModel", "text_model", "text-gemma4-12b"),
    ("RuntimeTextModel", "runtime_text_model", "text-gemma4-12b"),
    ("VoiceModel", "tts_model", "tts-cosyvoice3"),
    ("Swear", "swear", "false"),
    ("RuntimeGeneration", "runtime", "false"),
    ("AutomaticPreparation", "auto_daily", "true"),
    ("AutoKeepDaily", "daily_auto_keep", "true"),
];

pub fn reconcile(
    raw: &Map<String, Value>,
    database: &BTreeMap<String, Value>,
) -> Result<SettingsSnapshot, SettingsError> {
    if let Some(version) = raw.get("SchemaVersion") {
        match version.as_u64() {
            Some(0..=2) => {}
            Some(_) => return Err(SettingsError::FutureSchema),
            None => return Err(SettingsError::Invalid("SchemaVersion".into())),
        }
    }
    let mut desktop: BTreeMap<_, _> = raw
        .iter()
        .filter(|(key, _)| DESKTOP_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let overrides = raw
        .get("BackendSettingsOverrides")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for &(desktop_key, backend_key, fallback) in SHARED {
        let flag = if matches!(backend_key, "auto_daily" | "daily_auto_keep") {
            "BackendDailySettingsMigrated"
        } else {
            "BackendSettingsMigrated"
        };
        let migrated = raw.get(flag).and_then(Value::as_bool).unwrap_or(false);
        let manual = overrides
            .iter()
            .any(|value| value.as_str() == Some(backend_key));
        let selected = if !migrated && !manual {
            database.get(backend_key).or_else(|| raw.get(desktop_key))
        } else {
            raw.get(desktop_key).or_else(|| database.get(backend_key))
        };
        let value = selected.cloned().unwrap_or_else(|| match fallback {
            "true" => json!(true),
            "false" => json!(false),
            _ => json!(fallback),
        });
        if matches!(
            backend_key,
            "swear" | "runtime" | "auto_daily" | "daily_auto_keep"
        ) {
            if !value.is_boolean() {
                return Err(SettingsError::Invalid(desktop_key.into()));
            }
        } else if !value.is_string() {
            return Err(SettingsError::Invalid(desktop_key.into()));
        }
        desktop.insert(desktop_key.into(), value);
    }
    // These modern desktop values were sent back to the legacy worker on startup.
    // Keep that precedence, but do not throw away a database-only preference.
    for (desktop_key, backend_key) in [
        ("SpeechLanguage", "speech_language"),
        ("VoiceProfile", "voice_profile"),
        ("AddressForm", "address_form"),
        ("UserName", "user_name"),
    ] {
        if !desktop.contains_key(desktop_key)
            && let Some(value) = database.get(backend_key)
        {
            desktop.insert(desktop_key.into(), value.clone());
        }
    }
    let mut inference: BTreeMap<_, _> = database
        .iter()
        .filter(|(key, _)| INFERENCE_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for &(_, key, _) in SHARED {
        inference.remove(key);
    }
    for key in [
        "speech_language",
        "voice_profile",
        "address_form",
        "user_name",
    ] {
        inference.remove(key);
    }
    // Never discard a legacy non-default description: it may not yet have been
    // captured into a character preset. The preset selection remains independent.
    let result = SettingsSnapshot {
        format: 1,
        desktop,
        inference,
        ignored_desktop_fields: raw
            .keys()
            .filter(|key| !DESKTOP_FIELDS.contains(&key.as_str()))
            .cloned()
            .collect(),
        ignored_inference_fields: database
            .keys()
            .filter(|key| !INFERENCE_FIELDS.contains(&key.as_str()))
            .cloned()
            .collect(),
    };
    validate(&result)?;
    Ok(result)
}

#[cfg(test)]
#[path = "legacy_contract_tests.rs"]
mod contract_tests;

fn validate(snapshot: &SettingsSnapshot) -> Result<(), SettingsError> {
    let d = &snapshot.desktop;
    for (key, min, max) in [
        ("WorkStart", 0, 1439),
        ("WorkEnd", 0, 1439),
        ("BreakEvery", 5, 720),
        ("BreakfastTime", 0, 1439),
        ("LunchTime", 0, 1439),
        ("DinnerTime", 0, 1439),
        ("IdleMinutes", 1, 120),
        ("AllowedClicks", 0, 100),
        ("RepeatMinutes", 1, 120),
        ("BreakRepeats", 0, 20),
        ("ExerciseRepeats", 0, 20),
        ("MealRepeats", 0, 20),
        ("ExerciseAfter", 1, 1440),
    ] {
        if let Some(value) = d.get(key) {
            let Some(number) = value.as_i64() else {
                return Err(SettingsError::Invalid(key.into()));
            };
            if !(min..=max).contains(&number) {
                return Err(SettingsError::Invalid(key.into()));
            }
        }
    }
    if let (Some(start), Some(end)) = (d.get("WorkStart"), d.get("WorkEnd"))
        && start.as_i64() > end.as_i64()
    {
        return Err(SettingsError::Invalid("WorkEnd".into()));
    }
    if let Some(value) = d.get("WorkDays") {
        let Some(days) = value.as_array() else {
            return Err(SettingsError::Invalid("WorkDays".into()));
        };
        let mut seen = Vec::new();
        for day in days {
            let Some(day) = day.as_u64().filter(|n| *n <= 6) else {
                return Err(SettingsError::Invalid("WorkDays".into()));
            };
            if seen.contains(&day) {
                return Err(SettingsError::Invalid("WorkDays".into()));
            }
            seen.push(day);
        }
    }
    for (key, allowed) in [
        ("UiLanguage", &["ru", "en"][..]),
        ("SpeechLanguage", &["ru", "en"][..]),
        ("ThemeMode", &["system", "light", "dark"][..]),
        ("AddressForm", &["neutral", "female", "male"][..]),
    ] {
        if let Some(value) = d.get(key)
            && !value.as_str().is_some_and(|s| allowed.contains(&s))
        {
            return Err(SettingsError::Invalid(key.into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }
    #[test]
    fn incomplete_import_respects_manual_changes_and_independent_daily_flag() {
        let desktop = object(json!({"TextModel":"text-qwen35-4b", "Swear":false,
            "BackendSettingsOverrides":["text_model"], "AutomaticPreparation":false,
            "BackendDailySettingsMigrated":true}));
        let db = BTreeMap::from([
            ("text_model".into(), json!("text-gemma4-12b")),
            ("swear".into(), json!(true)),
            ("auto_daily".into(), json!(true)),
        ]);
        let result = reconcile(&desktop, &db).unwrap();
        assert_eq!(result.desktop["TextModel"], "text-qwen35-4b");
        assert_eq!(result.desktop["Swear"], true);
        assert_eq!(result.desktop["AutomaticPreparation"], false);
        assert!(!result.inference.contains_key("text_model"));
    }
    #[test]
    fn completed_import_keeps_desktop_and_preserves_advanced_private_choices() {
        let desktop = object(json!({"BackendSettingsMigrated":true,
            "TextModel":"text-saiga-llama3-8b", "TimeZoneId":"Central Europe Standard Time",
            "ExerciseUtc":"/Date(1792756800000)/", "History":["Отдохни."],
            "ApiKey":"not-a-real-key"}));
        let db = BTreeMap::from([
            ("text_model".into(), json!("text-gemma4-12b")),
            (
                "model_paths".into(),
                json!({"text-saiga-llama3-8b":"E:/Модели/voice"}),
            ),
            ("cosy_instruction".into(), json!("Спокойно")),
            ("api_token".into(), json!("not-a-real-token")),
        ]);
        let result = reconcile(&desktop, &db).unwrap();
        assert_eq!(result.desktop["TextModel"], "text-saiga-llama3-8b");
        assert_eq!(result.desktop["ExerciseUtc"], desktop["ExerciseUtc"]);
        assert_eq!(result.inference["model_paths"], db["model_paths"]);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("not-a-real-")
        );
    }
    #[test]
    fn future_or_corrupt_settings_are_not_silently_reset() {
        assert!(matches!(
            reconcile(&object(json!({"SchemaVersion":3})), &BTreeMap::new()),
            Err(SettingsError::FutureSchema)
        ));
        assert!(reconcile(&object(json!({"IdleMinutes":0})), &BTreeMap::new()).is_err());
        assert!(reconcile(&object(json!({"WorkDays":[1,1]})), &BTreeMap::new()).is_err());
    }
}
