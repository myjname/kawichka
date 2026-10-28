//! New UI preferences. Legacy conversion remains an explicit, separate operation.
use crate::schedule::Schedule;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Ru,
    En,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VisualMode {
    Both,
    Message,
    #[default]
    Nothing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioMode {
    #[default]
    All,
    Chime,
    Voice,
    Silent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnimationIntensity {
    Off,
    #[default]
    Subtle,
    Expressive,
}
impl AnimationIntensity {
    /// Compatibility gate for saved intensity choices, including legacy Off.
    pub fn idle_active(self, enabled: bool, platform_allowed: bool, paused: bool) -> bool {
        enabled && platform_allowed && !paused && self != Self::Off
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReminderOptions {
    pub allow_during_dnd: bool,
    pub break_repeats: u32,
    pub exercise_repeats: u32,
    pub meal_repeats: u32,
    pub pet_visible: bool,
    pub animation: bool,
    pub animation_intensity: AnimationIntensity,
    pub idle_continuous: bool,
    pub idle_play_seconds: u16,
    pub idle_pause_seconds: u16,
    /// Empty selects the system default. A missing pinned ID is never replaced.
    pub audio_device: String,
    pub audio: AudioMode,
    pub fullscreen_visual: VisualMode,
    pub fullscreen_audio: AudioMode,
}
impl Default for ReminderOptions {
    fn default() -> Self {
        Self {
            allow_during_dnd: false,
            break_repeats: 3,
            exercise_repeats: 3,
            meal_repeats: 0,
            pet_visible: true,
            animation: true,
            animation_intensity: AnimationIntensity::Subtle,
            idle_continuous: false,
            idle_play_seconds: 5,
            idle_pause_seconds: 30,
            audio_device: String::new(),
            audio: AudioMode::All,
            fullscreen_visual: VisualMode::Nothing,
            fullscreen_audio: AudioMode::Silent,
        }
    }
}
impl ReminderOptions {
    /// One policy for stock and custom mascots. Reactions bypass idle intervals,
    /// but never bypass Off, application pause or the system motion policy.
    pub fn animation_active(
        &self,
        platform_allowed: bool,
        paused: bool,
        elapsed_ms: u64,
        reaction: bool,
    ) -> bool {
        if !self
            .animation_intensity
            .idle_active(self.animation, platform_allowed, paused)
        {
            return false;
        }
        let play = u64::from(self.idle_play_seconds) * 1000;
        let pause = u64::from(self.idle_pause_seconds) * 1000;
        reaction || self.idle_continuous || elapsed_ms % (play + pause).max(1) < play
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub format: u32,
    pub theme: Theme,
    pub ui_language: Language,
    pub speech_language: Language,
    pub user_name: String,
    pub schedule: Schedule,
    pub idle_minutes: u16,
    pub allowed_clicks: u16,
    pub repeat_minutes: u16,
    pub trust_confirmation: bool,
    #[serde(default)]
    pub reminders: ReminderOptions,
    #[serde(default)]
    pub generation: crate::generation::Generation,
    #[serde(default)]
    pub desktop: DesktopOptions,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopOptions {
    pub time_zone: String,
    pub character_preset: String,
    pub visual_preset: String,
    pub visual_revision: u64,
    pub pet_size: u16,
    pub defer_generation_fullscreen: bool,
    pub run_at_login: bool,
    pub automatic_updates: bool,
    pub setup_complete: bool,
    pub last_notified_update: String,
    pub recovery_limit_bytes: u64,
}
impl Default for DesktopOptions {
    fn default() -> Self {
        Self {
            time_zone: String::new(),
            character_preset: String::new(),
            visual_preset: String::new(),
            visual_revision: 0,
            pet_size: 130,
            defer_generation_fullscreen: true,
            run_at_login: false,
            automatic_updates: true,
            setup_complete: false,
            last_notified_update: String::new(),
            recovery_limit_bytes: 2_000_000_000,
        }
    }
}
impl DesktopOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(80..=260).contains(&self.pet_size)
            || self.time_zone.len() > 128
            || self.time_zone.chars().any(char::is_control)
            || [&self.character_preset, &self.visual_preset]
                .iter()
                .any(|s| s.len() > 128 || !s.chars().all(|c| c.is_ascii_alphanumeric()))
            || self.last_notified_update.len() > 100
            || !(100_000_000..=100_000_000_000).contains(&self.recovery_limit_bytes)
        {
            return Err("invalid desktop options".into());
        }
        Ok(())
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            format: 1,
            theme: Theme::System,
            ui_language: Language::Ru,
            speech_language: Language::Ru,
            user_name: String::new(),
            schedule: Schedule::default(),
            idle_minutes: 5,
            allowed_clicks: 3,
            repeat_minutes: 5,
            trust_confirmation: false,
            reminders: ReminderOptions::default(),
            generation: crate::generation::Generation::default(),
            desktop: DesktopOptions::default(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1 {
            return Err("unsupported preferences version".into());
        }
        self.schedule.validate().map_err(|e| e.to_string())?;
        self.generation.validate()?;
        self.desktop.validate()?;
        if !(1..=120).contains(&self.idle_minutes)
            || self.allowed_clicks > 100
            || !(1..=120).contains(&self.repeat_minutes)
            || self.user_name.chars().count() > 80
            || self.user_name.chars().any(char::is_control)
            || self.reminders.audio_device.len() > 1024
            || self.reminders.audio_device.chars().any(char::is_control)
            || !(1..=300).contains(&self.reminders.idle_play_seconds)
            || !(1..=3600).contains(&self.reminders.idle_pause_seconds)
            || [
                self.reminders.break_repeats,
                self.reminders.exercise_repeats,
                self.reminders.meal_repeats,
            ]
            .iter()
            .any(|v| *v > 20)
        {
            return Err("invalid activity settings or user name".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod shell_options_tests {
    use super::*;
    #[test]
    fn configured_idle_cycle_reactions_and_hard_stops() {
        let mut options = ReminderOptions {
            idle_play_seconds: 7,
            idle_pause_seconds: 13,
            ..Default::default()
        };
        for (time, active) in [
            (0, true),
            (6999, true),
            (7000, false),
            (19999, false),
            (20000, true),
        ] {
            assert_eq!(options.animation_active(true, false, time, false), active);
        }
        assert!(options.animation_active(true, false, 8000, true));
        options.idle_continuous = true;
        assert!(options.animation_active(true, false, 8000, false));
        for continuous in [false, true] {
            options.idle_continuous = continuous;
            for reaction in [false, true] {
                assert!(!options.animation_active(false, false, 0, reaction));
                assert!(!options.animation_active(true, true, 0, reaction));
                options.animation = false;
                assert!(!options.animation_active(true, false, 0, reaction));
                options.animation = true;
                options.animation_intensity = AnimationIntensity::Off;
                assert!(!options.animation_active(true, false, 0, reaction));
                options.animation_intensity = AnimationIntensity::Subtle;
            }
        }
    }
    #[test]
    fn idle_timing_defaults_migration_validation_and_roundtrip() {
        let mut raw = serde_json::to_value(Preferences::default()).unwrap();
        let reminders = raw["reminders"].as_object_mut().unwrap();
        for field in ["idle_continuous", "idle_play_seconds", "idle_pause_seconds"] {
            reminders.remove(field);
        }
        reminders.insert("animation_intensity".into(), "off".into());
        let mut prefs: Preferences = serde_json::from_value(raw).unwrap();
        assert!(!prefs.reminders.idle_continuous);
        assert_eq!(prefs.reminders.idle_play_seconds, 5);
        assert_eq!(prefs.reminders.idle_pause_seconds, 30);
        assert!(!prefs.reminders.animation_active(true, false, 0, true));
        for (play, pause) in [(0, 30), (301, 30), (5, 0), (5, 3601)] {
            prefs.reminders.idle_play_seconds = play;
            prefs.reminders.idle_pause_seconds = pause;
            assert!(prefs.validate().is_err());
        }
        prefs.reminders.idle_play_seconds = 300;
        prefs.reminders.idle_pause_seconds = 3600;
        prefs.reminders.idle_continuous = true;
        prefs.validate().unwrap();
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(prefs).unwrap()
        );
    }
    #[test]
    fn continuous_idle_respects_off_pause_and_platform_motion_policy() {
        for mode in [AnimationIntensity::Subtle, AnimationIntensity::Expressive] {
            assert!(mode.idle_active(true, true, false));
            assert!(!mode.idle_active(false, true, false));
            assert!(!mode.idle_active(true, false, false));
            assert!(!mode.idle_active(true, true, true));
        }
        assert!(!AnimationIntensity::Off.idle_active(true, true, false));
    }
    #[test]
    fn old_preferences_gain_safe_defaults_and_draft_preserves_new_choices() {
        let mut old = serde_json::to_value(Preferences::default()).unwrap();
        old["reminders"]
            .as_object_mut()
            .unwrap()
            .remove("audio_device");
        old["reminders"]
            .as_object_mut()
            .unwrap()
            .remove("animation_intensity");
        let base: Preferences = serde_json::from_value(old).unwrap();
        assert_eq!(
            base.reminders.animation_intensity,
            AnimationIntensity::Subtle
        );
        assert!(base.reminders.audio_device.is_empty());
        let mut current = base.clone();
        current.reminders.audio_device = "pinned-endpoint".into();
        current.reminders.animation_intensity = AnimationIntensity::Off;
        let mut draft = base.clone();
        draft.user_name = "Changed name".into();
        let merged = merge(&current, &base, &draft).unwrap();
        assert_eq!(merged.reminders.audio_device, "pinned-endpoint");
        assert_eq!(
            merged.reminders.animation_intensity,
            AnimationIntensity::Off
        );
        assert_eq!(merged.user_name, "Changed name");
    }
    #[test]
    fn every_visual_obeys_pause_reduce_motion_power_and_intensity() {
        for mode in [
            AnimationIntensity::Off,
            AnimationIntensity::Subtle,
            AnimationIntensity::Expressive,
        ] {
            let mut options = ReminderOptions {
                animation_intensity: mode,
                ..Default::default()
            };
            for reaction in [false, true] {
                assert!(!options.animation_active(false, false, 0, reaction));
                assert!(!options.animation_active(true, true, 0, reaction));
                options.animation = false;
                assert!(!options.animation_active(true, false, 0, reaction));
                options.animation = true;
            }
        }
        let options = ReminderOptions::default();
        assert!(options.animation_active(true, false, 0, false));
        assert!(!options.animation_active(true, false, 5000, false));
        assert!(options.animation_active(true, false, 5000, true));
    }
}
/// A settings draft must not overwrite unrelated tray changes made after it opened.
pub fn merge(
    current: &Preferences,
    base: &Preferences,
    desired: &Preferences,
) -> Result<Preferences, String> {
    fn patch(
        current: &mut serde_json::Value,
        base: &serde_json::Value,
        desired: &serde_json::Value,
    ) {
        if let (Some(c), Some(b), Some(d)) = (
            current.as_object_mut(),
            base.as_object(),
            desired.as_object(),
        ) {
            for (key, value) in d {
                if let Some(old) = b.get(key) {
                    if let Some(target) = c.get_mut(key) {
                        patch(target, old, value);
                    } else if old != value {
                        c.insert(key.clone(), value.clone());
                    }
                } else {
                    c.insert(key.clone(), value.clone());
                }
            }
            for key in b.keys().filter(|key| !d.contains_key(*key)) {
                c.remove(key);
            }
        } else if base != desired {
            *current = desired.clone();
        }
    }
    let mut current = serde_json::to_value(current).map_err(|e| e.to_string())?;
    patch(
        &mut current,
        &serde_json::to_value(base).map_err(|e| e.to_string())?,
        &serde_json::to_value(desired).map_err(|e| e.to_string())?,
    );
    let merged: Preferences = serde_json::from_value(current).map_err(|e| e.to_string())?;
    merged.validate()?;
    Ok(merged)
}

#[cfg(test)]
mod draft_map_tests {
    use super::*;
    #[test]
    fn adding_and_removing_a_selected_model_path_preserves_other_draft_and_concurrent_paths() {
        let mut base = Preferences::default();
        base.generation
            .model_paths
            .insert("text-gemma4-12b".into(), "E:\\old".into());
        base.generation
            .model_paths
            .insert("tts-0.6b".into(), "E:\\voice".into());
        let mut current = base.clone();
        current
            .generation
            .model_paths
            .insert("text-qwen35-4b".into(), "E:\\concurrent".into());
        current
            .generation
            .model_paths
            .insert("tts-0.6b".into(), "E:\\changed voice".into());
        let mut desired = base.clone();
        desired.generation.model_paths.remove("text-gemma4-12b");
        desired
            .generation
            .model_paths
            .insert("tts-1.7b".into(), "E:\\new".into());
        let result = merge(&current, &base, &desired).unwrap();
        assert!(
            !result
                .generation
                .model_paths
                .contains_key("text-gemma4-12b")
        );
        assert_eq!(result.generation.model_paths["tts-1.7b"], "E:\\new");
        assert_eq!(
            result.generation.model_paths["text-qwen35-4b"],
            "E:\\concurrent"
        );
        assert_eq!(
            result.generation.model_paths["tts-0.6b"],
            "E:\\changed voice"
        );
    }
}
