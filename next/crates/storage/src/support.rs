//! Daily encouragement is text-only and never enters the reminder/audio queue.
use crate::sandbox::Sandbox;
use kavychka_core::preferences::{Language, Preferences};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub format: u32,
    pub day: String,
    pub identity: String,
    pub texts: Vec<String>,
    pub generated: bool,
    pub attempted: bool,
}
pub fn identity(p: &Preferences, day: &str) -> String {
    json!([
        "support-v1",
        day,
        p.speech_language,
        p.user_name,
        p.generation.character_description,
        p.generation.text_model,
        p.generation.text_enabled,
        p.generation.address_form,
        p.generation.swear
    ])
    .to_string()
}
pub fn load(store: &Sandbox, p: &Preferences, day: &str) -> Result<Day, String> {
    let identity = identity(p, day);
    if let Some(raw) = store.meta("support_day").map_err(|e| e.to_string())? {
        validate_value("support_day", &raw)?;
        let saved: Day = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        if saved.identity == identity && valid(&saved.texts) {
            return Ok(saved);
        }
    }
    const RU: [&str; 12] = [
        "Тебе не нужно успеть всё сразу. Одного небольшого шага сейчас достаточно.",
        "Сложный день не отменяет того, сколько уже получилось сделать.",
        "Я рядом. Можно выдохнуть и выбрать только одну следующую задачу.",
        "Небольшой прогресс тоже считается. Даже если никто не выдаёт за него медаль.",
        "Сегодня можно беречь силы. Они пригодятся тебе больше, чем идеальный список дел.",
        "Не каждый час обязан быть продуктивным. Тебе можно просто побыть собой.",
        "Ты справляешься в своём темпе. Я не собираюсь сверять тебя с чужим секундомером.",
        "Если задача кажется огромной, начни с самого маленького края.",
        "Право на отдых не нужно зарабатывать количеством закрытых задач.",
        "У тебя уже есть опыт проходить непростые дни. Этот тоже не бесконечный.",
        "Можно передумать и выбрать более лёгкий путь. Это тоже решение.",
        "Не всё должно получиться с первого раза. Оставим немного места для второй попытки.",
    ];
    const EN: [&str; 12] = [
        "You do not have to finish everything at once. One small step is enough for now.",
        "A difficult day does not erase how much you have already done.",
        "I am here. Take a breath and pick just one next task.",
        "Small progress still counts, even without a medal.",
        "You can save your energy today. It matters more than a perfect task list.",
        "Not every hour has to be productive. You are allowed to simply be yourself.",
        "Your pace is yours. I am not timing you against somebody else.",
        "If the task looks enormous, start at its smallest edge.",
        "Rest is not something you have to earn by completing tasks.",
        "You have made it through difficult days before. This one is not endless either.",
        "Changing your mind and taking an easier route is a valid decision.",
        "It does not all have to work on the first try. There is room for another attempt.",
    ];
    let pool = if p.speech_language == Language::En {
        &EN
    } else {
        &RU
    };
    let rotation = day
        .bytes()
        .fold(0usize, |v, b| v.wrapping_mul(31).wrapping_add(b as usize));
    let texts = (0..4)
        .map(|i| pool[(rotation + i * 3) % pool.len()].to_string())
        .collect();
    let value = Day {
        format: 1,
        day: day.into(),
        identity,
        texts,
        generated: false,
        attempted: false,
    };
    save(store, &value)?;
    Ok(value)
}
pub fn valid(texts: &[String]) -> bool {
    (1..=8).contains(&texts.len())
        && texts
            .iter()
            .all(|s| (8..=320).contains(&s.chars().count()) && !s.chars().any(|c| c.is_control()))
}
pub fn validate_value(key: &str, value: &serde_json::Value) -> Result<(), String> {
    if key == "support_day" {
        let day: Day = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if day.format != 1
            || day.day.parse::<chrono::NaiveDate>().is_err()
            || day.identity.len() > 40000
            || !valid(&day.texts)
        {
            return Err("Invalid daily encouragement metadata".into());
        }
    } else if key == "support_history" {
        let texts: Vec<String> =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if texts.len() > 64 || texts.iter().any(|s| !valid(std::slice::from_ref(s))) {
            return Err("Invalid encouragement history".into());
        }
    } else {
        return Err("Unknown encouragement metadata".into());
    }
    Ok(())
}
pub fn save(store: &Sandbox, day: &Day) -> Result<(), String> {
    validate_value("support_day", &json!(day))?;
    store
        .put_meta("support_day", &json!(day))
        .map_err(|e| e.to_string())
}
pub fn history(store: &Sandbox) -> Result<Vec<String>, String> {
    store
        .meta("support_history")
        .map_err(|e| e.to_string())?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())
        .map(Option::unwrap_or_default)
}
pub fn accept(store: &Sandbox, day: &mut Day, texts: Vec<String>) -> Result<(), String> {
    if !valid(&texts) {
        return Err("Invalid generated encouragement texts".into());
    }
    let mut history = history(store)?;
    for text in &texts {
        if history.contains(text) || texts.iter().filter(|v| *v == text).count() > 1 {
            return Err("Repeated encouragement text".into());
        }
    }
    history.extend(texts.iter().cloned());
    if history.len() > 64 {
        history.drain(..history.len() - 64);
    }
    let mut next = day.clone();
    next.texts = texts;
    next.generated = true;
    validate_value("support_day", &json!(next))?;
    let tx = store
        .connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    for (key, value) in [
        ("support_history", json!(history)),
        ("support_day", json!(next)),
    ] {
        tx.execute("INSERT INTO library_meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET json=excluded.json",rusqlite::params![key,value.to_string()]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    *day = next;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daily_support_survives_restart_and_never_creates_phrase_rows_or_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = Sandbox::open(dir.path()).unwrap();
        let prefs = Preferences::default();
        let mut day = load(&store, &prefs, "2026-10-28").unwrap();
        let built_in = day.texts.clone();
        day.attempted = true;
        save(&store, &day).unwrap();
        assert_eq!(load(&store, &prefs, "2026-10-28").unwrap().texts, built_in);
        assert!(load(&store, &prefs, "2026-10-28").unwrap().attempted);
        accept(
            &store,
            &mut day,
            vec![
                "Сегодня можно двигаться своим темпом.".into(),
                "Небольшие победы тоже считаются.".into(),
            ],
        )
        .unwrap();
        assert!(load(&store, &prefs, "2026-10-28").unwrap().generated);
        assert!(
            accept(
                &store,
                &mut day,
                vec!["Небольшие победы тоже считаются.".into()]
            )
            .is_err()
        );
        let next = load(&store, &prefs, "2026-10-29").unwrap();
        assert!(!next.attempted);
        assert_ne!(next.texts, built_in);
        assert_eq!(history(&store).unwrap().len(), 2);
        assert!(store.phrases().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(store.root.join("audio")).unwrap().count(),
            0
        );
        let mut other = prefs.clone();
        other.generation.character_description = "A different character".into();
        assert_ne!(
            load(&store, &other, "2026-10-29").unwrap().identity,
            next.identity
        );
    }
}
