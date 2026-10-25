//! The existing public neutral phrase bank, ported from Fallbacks.cs.
use serde_json::Value;
use std::sync::OnceLock;
pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace('ё', "е")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn candidates(
    language: &str,
    category: &str,
    followup: bool,
    seed: u64,
) -> impl Iterator<Item = String> {
    static BANK: OnceLock<Value> = OnceLock::new();
    let bank = &BANK.get_or_init(|| {
        serde_json::from_str(include_str!("builtin-phrases.json"))
            .expect("checked public stock bank")
    })[language];
    let key = if followup {
        if category == "break" {
            "followup".into()
        } else {
            format!("{category}_followup")
        }
    } else {
        category.to_string()
    };
    let bodies = bank["banks"]
        .get(&key)
        .unwrap_or(&bank["banks"]["break"])
        .as_array()
        .expect("stock bodies");
    let leads = bank["leads"].as_array().expect("stock leads");
    let tails = bank["tails"].as_array().expect("stock tails");
    let (b, l, t) = (bodies.len(), leads.len(), tails.len());
    (0..b * (1 + l + l * t)).map(move |i| {
        let body = bodies[(seed as usize % b + i % b) % b].as_str().unwrap();
        if i < b {
            return body.to_owned();
        }
        let j = i - b;
        let lead = leads[(seed as usize % l + j / b % l) % l].as_str().unwrap();
        if j < l * b {
            return format!("{lead} {body}");
        }
        let tail = tails[(seed as usize % t + (j - l * b) / (l * b)) % t]
            .as_str()
            .unwrap();
        format!("{lead} {body} {tail}")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_banks_cover_both_languages_and_retain_distinct_stock_candidates() {
        for lang in ["ru", "en"] {
            for category in crate::generation::CATEGORIES {
                for followup in [false, true] {
                    let texts = candidates(lang, category, followup, 23)
                        .take(20)
                        .collect::<Vec<_>>();
                    assert!(
                        texts
                            .iter()
                            .all(|t| !t.trim().is_empty() && t.chars().count() <= 400)
                    );
                    let unique = texts
                        .iter()
                        .map(|t| normalize(t))
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(unique.len(), texts.len());
                }
            }
        }
    }
}
