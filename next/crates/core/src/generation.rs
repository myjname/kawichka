//! Versioned inference choices. Workers receive a snapshot, never a database.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const TEXT_MODELS: &[&str] = &[
    "text-gemma4-12b",
    "text-gemma4-e4b",
    "text-qwen35-4b",
    "text-qwen35-9b",
    "text-ministral3-8b",
    "text-qvikhr3-8b",
    "text-saiga-llama3-8b",
    "text-1.7b",
];
pub const VOICE_MODELS: &[&str] = &["system", "tts-cosyvoice3", "tts-0.6b", "tts-1.7b"];
pub const CATEGORIES: &[&str] = &["break", "breakfast", "lunch", "dinner", "exercise"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Generation {
    pub text_model: String,
    pub runtime_text_model: String,
    pub tts_model: String,
    pub runtime: bool,
    pub text_enabled: bool,
    pub auto_daily: bool,
    pub daily_auto_keep: bool,
    pub batch_count: u16,
    pub categories: Vec<String>,
    pub device: String,
    pub model_paths: BTreeMap<String, String>,
    pub character_description: String,
    pub voice_profile: String,
    pub address_form: String,
    pub swear: bool,
    pub cosy_style: String,
    pub cosy_instruction: String,
    pub delivery_catalog: Value,
    pub runtime_wait_seconds: u16,
    pub ac_only: bool,
}
impl Default for Generation {
    fn default() -> Self {
        Self {
            text_model: "text-qwen35-4b".into(), runtime_text_model: "text-qwen35-4b".into(),
            tts_model: "system".into(), runtime: false, text_enabled: false,
            auto_daily: true, daily_auto_keep: true, batch_count: 13,
            categories: CATEGORIES.iter().map(|s| (*s).into()).collect(), device: "auto".into(),
            model_paths: BTreeMap::new(),
            character_description: "Ты — Кавычка, спокойный и остроумный компаньон. Напоминай о еде и отдыхе коротко, разнообразно, с лёгкой усмешкой, без давления и повторяющихся шаблонов.".into(),
            voice_profile: String::new(), address_form: "neutral".into(), swear: false,
            cosy_style: "auto".into(), cosy_instruction: String::new(),
            delivery_catalog: json!([
                {"id":"confident","name":"Уверенно","instruction":"Speak confidently and firmly, unhurriedly, with clear emphasis."},
                {"id":"calm","name":"Спокойно","instruction":"Speak calmly and unhurriedly, with a relaxed low voice."},
                {"id":"amused","name":"С лёгкой усмешкой","instruction":"Speak with gentle amusement and a subtle smile, lightly teasing."}
            ]), runtime_wait_seconds: 10, ac_only: true,
        }
    }
}
impl Generation {
    pub fn validate(&self) -> Result<(), String> {
        if !TEXT_MODELS.contains(&self.text_model.as_str())
            || !TEXT_MODELS.contains(&self.runtime_text_model.as_str())
            || !VOICE_MODELS.contains(&self.tts_model.as_str())
            || !(1..=100).contains(&self.batch_count)
            || !(1..=120).contains(&self.runtime_wait_seconds)
            || !["auto", "cpu", "cuda"].contains(&self.device.as_str())
            || !["neutral", "female", "male"].contains(&self.address_form.as_str())
            || self.categories.is_empty()
            || self
                .categories
                .iter()
                .any(|s| !CATEGORIES.contains(&s.as_str()))
            || self.character_description.chars().count() > 4000
            || self.cosy_instruction.chars().count() > 600
            || self.voice_profile.len() > 128
            || !self
                .voice_profile
                .chars()
                .all(|c| c.is_ascii_alphanumeric())
            || self.model_paths.iter().any(|(id, p)| {
                !TEXT_MODELS.contains(&id.as_str()) && !VOICE_MODELS.contains(&id.as_str())
                    || !std::path::Path::new(p).is_absolute()
            })
        {
            return Err("invalid generation settings".into());
        }
        let styles = self
            .delivery_catalog
            .as_array()
            .ok_or("invalid delivery catalog")?;
        let mut ids = std::collections::BTreeSet::new();
        if styles.is_empty() || styles.len() > 40 {
            return Err("invalid delivery count".into());
        }
        for item in styles {
            let id = item["id"].as_str().ok_or("invalid delivery id")?;
            let name = item["name"].as_str().ok_or("invalid delivery name")?;
            let instruction = item["instruction"]
                .as_str()
                .ok_or("invalid delivery instruction")?;
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                || !id.as_bytes()[0].is_ascii_lowercase()
                || ["auto", "custom"].contains(&id)
                || !ids.insert(id)
                || name.trim().is_empty()
                || name.chars().count() > 60
                || instruction.trim().is_empty()
                || instruction.chars().count() > 600
                || instruction.contains("<|")
            {
                return Err("invalid delivery catalog entry".into());
            }
        }
        if !["auto", "custom"].contains(&self.cosy_style.as_str())
            && !ids.contains(self.cosy_style.as_str())
            || self.cosy_style == "custom" && self.cosy_instruction.trim().is_empty()
            || self.cosy_instruction.contains("<|")
        {
            return Err("invalid delivery selection".into());
        }
        Ok(())
    }
    pub fn worker_settings(&self, language: &str, user_name: &str) -> Value {
        let mut value = serde_json::to_value(self).expect("generation serializes");
        value["speech_language"] = json!(language);
        value["user_name"] = json!(user_name);
        value
    }
}
