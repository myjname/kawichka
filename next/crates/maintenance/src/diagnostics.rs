//! Allowlisted diagnostics. Never copies logs, SQL, names, prompts or paths.
use crate::Result;
use kavychka_storage::sandbox::Sandbox;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format: u32,
    pub app_version: String,
    pub platform: String,
    pub working_schema: u32,
    pub rows: usize,
    pub recordings: usize,
    pub approved: usize,
    pub excluded: usize,
    pub runtime: bool,
    pub local_text: bool,
    pub voice_model: String,
    pub device: String,
    pub setup_complete: bool,
    pub checks: Vec<Check>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub code: String,
    pub cause_ru: String,
    pub action_ru: String,
    pub cause_en: String,
    pub action_en: String,
}
pub fn capture(store: &Sandbox, version: &str) -> Result<Report> {
    crate::package::version(version)?;
    let p = store.load()?;
    let rows = store.phrases()?;
    let missing = rows
        .iter()
        .filter(|r| r.audio_path.is_some() && store.audio_file(r).ok().flatten().is_none())
        .count();
    let mut checks = vec![];
    if !p.desktop.setup_complete {
        checks.push(Check {
            code: "setup_incomplete".into(),
            cause_ru: "Первый запуск не завершён".into(),
            action_ru: "Заверши мастер, чтобы включить напоминания".into(),
            cause_en: "Setup is incomplete".into(),
            action_en: "Complete setup to enable reminders".into(),
        });
    }
    if missing > 0 {
        checks.push(Check {
            code: "recordings_unavailable".into(),
            cause_ru: "Часть записей недоступна".into(),
            action_ru: "Подключи диск или перегенерируй озвучку сохранённых текстов".into(),
            cause_en: "Some recordings are unavailable".into(),
            action_en: "Reconnect the data disk or regenerate speech for saved texts".into(),
        });
    }
    checks.push(Check {
        code: "windows_notification_signal".into(),
        cause_ru: "Windows не предоставляет гарантированный точный сигнал Не беспокоить".into(),
        action_ru: "Для гарантированной тишины используй паузу Кавычки".into(),
        cause_en: "Windows notification availability is not an exact Do Not Disturb signal".into(),
        action_en: "Use Kavychka Pause for guaranteed silence".into(),
    });
    Ok(Report {
        format: 1,
        app_version: version.into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        working_schema: 3,
        rows: rows.len(),
        recordings: rows.iter().filter(|r| r.audio_path.is_some()).count(),
        approved: rows.iter().filter(|r| r.approved).count(),
        excluded: rows.iter().filter(|r| r.rejected || r.deleted).count(),
        runtime: p.generation.runtime,
        local_text: p.generation.text_enabled,
        voice_model: p.generation.tts_model,
        device: p.generation.device,
        setup_complete: p.desktop.setup_complete,
        checks,
    })
}
pub fn export(report: &Report, path: &Path) -> Result<()> {
    kavychka_storage::preview::no_link_chain(path)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(&serde_json::to_vec_pretty(report)?)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_export_never_serializes_private_settings_paths_or_logs() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/diagnostic-tests");
        std::fs::create_dir_all(&root).unwrap();
        let t = tempfile::tempdir_in(root).unwrap();
        let mut s = kavychka_storage::sandbox::Sandbox::open(&t.path().join("data")).unwrap();
        let mut p = s.load().unwrap();
        p.user_name = "PRIVATE-NAME-DO-NOT-EXPORT".into();
        p.generation.character_description = "PRIVATE-PROMPT".into();
        s.save(&p).unwrap();
        std::fs::create_dir_all(s.root.join("logs")).unwrap();
        std::fs::write(s.root.join("logs/private.log"), "PRIVATE-SECRET").unwrap();
        let r = super::capture(&s, "0.1.2").unwrap();
        let out = t.path().join("report.json");
        super::export(&r, &out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(!text.contains("PRIVATE") && !text.contains(&t.path().display().to_string()));
        assert_eq!(r.rows, 0);
        assert!(super::export(&r, &out).is_err());
        let _: super::Report = serde_json::from_str(&text).unwrap();
    }
}
