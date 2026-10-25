//! Convert a reconciled legacy snapshot; unexposed fields remain in its archive.
use crate::{
    legacy::SettingsSnapshot,
    preferences::{AudioMode, Language, Preferences, Theme, VisualMode},
};
use serde_json::{Value, json};
pub fn preferences(snapshot: &SettingsSnapshot) -> Result<Preferences, String> {
    if snapshot.format != 1 {
        return Err("unsupported settings snapshot".into());
    }
    let d = &snapshot.desktop;
    let mut p = serde_json::to_value(Preferences::default()).map_err(|e| e.to_string())?;
    for (old, new) in [
        ("IdleMinutes", "idle_minutes"),
        ("AllowedClicks", "allowed_clicks"),
        ("RepeatMinutes", "repeat_minutes"),
        ("TrustConfirmation", "trust_confirmation"),
        ("UserName", "user_name"),
    ] {
        if let Some(v) = d.get(old) {
            p[new] = v.clone();
        }
    }
    for (old, new) in [
        ("WorkDays", "work_days"),
        ("WorkStart", "work_start"),
        ("WorkEnd", "work_end"),
        ("BreakEvery", "break_every"),
        ("MealsEnabled", "meals_enabled"),
        ("BreakfastTime", "breakfast"),
        ("LunchTime", "lunch"),
        ("DinnerTime", "dinner"),
        ("ExerciseEnabled", "exercise_enabled"),
        ("ExerciseAfter", "exercise_after_minutes"),
    ] {
        if let Some(v) = d.get(old) {
            p["schedule"][new] = v.clone();
        }
    }
    for (old, new) in [
        ("BreakRepeats", "break_repeats"),
        ("ExerciseRepeats", "exercise_repeats"),
        ("MealRepeats", "meal_repeats"),
        ("PetVisible", "pet_visible"),
    ] {
        if let Some(v) = d.get(old) {
            p["reminders"][new] = v.clone();
        }
    }
    let mut prefs: Preferences = serde_json::from_value(p).map_err(|e| e.to_string())?;
    // Legacy constructor defaults differ from the neutral fresh-install policy.
    prefs.reminders.fullscreen_visual = VisualMode::Both;
    prefs.reminders.fullscreen_audio = AudioMode::All;
    for (key, target) in [
        ("UiLanguage", &mut prefs.ui_language),
        ("SpeechLanguage", &mut prefs.speech_language),
    ] {
        if let Some(v) = d.get(key) {
            *target = match v.as_str() {
                Some("ru") => Language::Ru,
                Some("en") => Language::En,
                _ => return Err(format!("unsupported {key}")),
            };
        }
    }
    if let Some(v) = d.get("ThemeMode") {
        prefs.theme = match v.as_str() {
            Some("system") => Theme::System,
            Some("light") => Theme::Light,
            Some("dark") => Theme::Dark,
            _ => return Err("unsupported theme".into()),
        };
    }
    let visual = |v: &Value| match v.as_str() {
        Some("both") => Ok(VisualMode::Both),
        Some("message") => Ok(VisualMode::Message),
        Some("none") => Ok(VisualMode::Nothing),
        _ => Err("unsupported fullscreen visual"),
    };
    if let Some(v) = d.get("FullscreenVisual") {
        prefs.reminders.fullscreen_visual = visual(v)?;
    }
    if let Some(v) = d.get("FullscreenAudio") {
        prefs.reminders.fullscreen_audio = match v.as_str() {
            Some("all") => AudioMode::All,
            Some("signal") | Some("chime") => AudioMode::Chime,
            Some("voice") => AudioMode::Voice,
            Some("none") | Some("silent") => AudioMode::Silent,
            _ => return Err("unsupported fullscreen audio".into()),
        };
    }
    prefs.reminders.audio = match (
        d.get("Voice").and_then(Value::as_bool).unwrap_or(true),
        d.get("NotificationSound")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    ) {
        (true, true) => AudioMode::All,
        (true, false) => AudioMode::Voice,
        (false, true) => AudioMode::Chime,
        (false, false) => AudioMode::Silent,
    };
    let mut g = serde_json::to_value(&prefs.generation).map_err(|e| e.to_string())?;
    for (k, v) in &snapshot.inference {
        if g.get(k).is_some() {
            g[k] = v.clone();
        }
    }
    for (old, new) in [
        ("TextModel", "text_model"),
        ("RuntimeTextModel", "runtime_text_model"),
        ("VoiceModel", "tts_model"),
        ("RuntimeGeneration", "runtime"),
        ("AutomaticPreparation", "auto_daily"),
        ("AutoKeepDaily", "daily_auto_keep"),
        ("Swear", "swear"),
        ("VoiceProfile", "voice_profile"),
        ("AddressForm", "address_form"),
    ] {
        if let Some(v) = d.get(old) {
            g[new] = v.clone();
        }
    }
    g["text_enabled"] = d.get("TextGeneration").cloned().unwrap_or_else(|| {
        json!(
            d.get("VoiceProvider")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "System")
        )
    });
    prefs.generation = serde_json::from_value(g).map_err(|e| e.to_string())?;
    let mut extra = serde_json::to_value(&prefs.desktop).map_err(|e| e.to_string())?;
    for (old, new) in [
        ("TimeZoneId", "time_zone"),
        ("CharacterFile", "character_preset"),
        ("VisualFile", "visual_preset"),
        ("PetSize", "pet_size"),
        ("DeferGenerationFullscreen", "defer_generation_fullscreen"),
        ("RunAtLogin", "run_at_login"),
        ("AutomaticUpdates", "automatic_updates"),
        ("SetupComplete", "setup_complete"),
        ("LastNotifiedUpdate", "last_notified_update"),
    ] {
        if let Some(v) = d.get(old) {
            extra[new] = v.clone();
        }
    }
    prefs.desktop = serde_json::from_value(extra).map_err(|e| e.to_string())?;
    if d.get("VoiceProvider")
        .and_then(Value::as_str)
        .unwrap_or("System")
        == "System"
    {
        prefs.generation.tts_model = "system".into();
    }
    prefs.validate()?;
    Ok(prefs)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_local_choices_and_schedule_become_active_without_credentials() {
        let s=crate::legacy::reconcile(json!({"WorkStart":630,"WorkDays":[0,6],"ThemeMode":"dark","VoiceProvider":"Qwen local","VoiceProfile":"private01","Swear":true,"ApiKey":"secret"}).as_object().unwrap(),&Default::default()).unwrap();
        let p = preferences(&s).unwrap();
        assert_eq!(p.schedule.work_start, 630);
        assert_eq!(p.schedule.work_days, [0, 6]);
        assert_eq!(p.theme, Theme::Dark);
        assert!(p.generation.text_enabled);
        assert!(p.generation.swear);
        assert_eq!(p.generation.voice_profile, "private01");
        assert!(!serde_json::to_string(&p).unwrap().contains("secret"));
    }
    #[test]
    fn legacy_system_voice_does_not_accidentally_select_a_local_clone_model() {
        let s=crate::legacy::reconcile(json!({"VoiceProvider":"System","VoiceModel":"tts-1.7b","BackendSettingsMigrated":true,"TextGeneration":true}).as_object().unwrap(),&Default::default()).unwrap();
        let p = preferences(&s).unwrap();
        assert_eq!(p.generation.tts_model, "system");
        assert!(p.generation.text_enabled);
        assert_eq!(s.desktop["VoiceModel"], "tts-1.7b");
    }
}
