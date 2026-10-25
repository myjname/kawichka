//! Explicit opt-in verification with installed models and a separate owned library.
use crate::{
    inference::{self, Worker},
    models_library::Manager,
};
use kavychka_storage::{library::Phrase, preview, sandbox::Sandbox};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
pub(crate) fn run_job(
    app: &Path,
    store: &mut Sandbox,
    resources: &Path,
    mut request: Value,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    if let Some(rotation) = store.meta("voice_rotation")? {
        request["rotation"] = rotation;
    }
    let mut worker = Worker::start(app, &store.root, resources, request)?;
    let mut values = vec![];
    let started = Instant::now();
    let mut reported = Instant::now();
    loop {
        for value in worker.poll() {
            let value = value?;
            match value["kind"].as_str() {
                Some("error") => {
                    return Err(value["message"]
                        .as_str()
                        .unwrap_or("worker error")
                        .to_string()
                        .into());
                }
                Some("text") | Some("phrase") => {
                    let mut row: Phrase = serde_json::from_value(value["row"].clone())?;
                    if value["kind"] == "text" {
                        row.approved = false;
                    }
                    store.accept_phrase_and_rotation(
                        &row,
                        Some(&worker.root),
                        value.get("rotation").and_then(Value::as_u64),
                        None,
                    )?;
                }
                _ => (),
            };
            values.push(value);
        }
        if worker.done() {
            break;
        }
        if started.elapsed() > Duration::from_secs(1200) {
            return Err("real model check timed out".into());
        }
        if reported.elapsed() > Duration::from_secs(15) {
            println!(
                "Installed-model check running · {} seconds",
                started.elapsed().as_secs()
            );
            reported = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(values)
}
pub fn run(
    data: PathBuf,
    source: PathBuf,
    evidence: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&evidence)?;
    let before = preview::inventory(&source, &AtomicBool::new(false))?;
    let mut store = Sandbox::open(&data)?;
    let origin = store.bootstrap_legacy(&source, &AtomicBool::new(false))?;
    let original_count = store.phrases()?.len();
    let mut prefs = store.load()?;
    prefs.generation.auto_daily = false;
    prefs.generation.daily_auto_keep = true;
    prefs.generation.text_enabled = true;
    prefs.generation.batch_count = 1;
    store.save(&prefs)?;
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or("executable directory missing")?;
    let app = if dir.join("next/worker.py").is_file() {
        dir.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()?
    };
    let models = inference::models(&app, &source, &data, &prefs)?;
    if !models
        .iter()
        .filter(|m| m.id == prefs.generation.text_model || m.id == prefs.generation.tts_model)
        .all(|m| m.installed && m.environment)
    {
        return Err("selected installed models/runtimes were not recognized".into());
    }
    let clock = kavychka_platform::time::now();
    let mut request =
        inference::request(&prefs, &store.phrases()?, "probe", &clock.date.to_string());
    let probe = run_job(&app, &mut store, &source, request.clone())?;
    if !probe
        .iter()
        .any(|v| v["kind"] == "result" && v["models"].is_array())
    {
        return Err("component probe did not complete".into());
    }
    let mut install = request.clone();
    install["operation"] = json!("install");
    install["ids"] = json!([
        prefs.generation.text_model.clone(),
        prefs.generation.tts_model.clone()
    ]);
    let reused = run_job(&app, &mut store, &source, install)?;
    if !reused.iter().any(|v| {
        v["kind"] == "result" && v["downloaded_models"].as_array().is_some_and(Vec::is_empty)
    }) {
        return Err("installed component selection unexpectedly requested model downloads".into());
    }
    request["operation"] = json!("generate");
    request["tasks"] = json!([{"category":"break","followup":false}]);
    request["approved"] = json!(true);
    let generated = run_job(&app, &mut store, &source, request)?;
    let row: Phrase = serde_json::from_value(
        generated
            .iter()
            .find(|v| v["kind"] == "phrase")
            .ok_or("missing generated phrase")?["row"]
            .clone(),
    )?;
    let decoded = kavychka_platform::audio::decode_recording(
        &store.audio_file(&row)?.ok_or("missing generated audio")?,
        &|| false,
    )?;
    if decoded.len() < 8000 {
        return Err("generated recording is too short".into());
    }
    let old_audio = row.audio_path.clone();
    let old_seed = row.seed;
    let mut request =
        inference::request(&prefs, &store.phrases()?, "speech", &clock.date.to_string());
    request["rows"] = json!([row.clone()]);
    let rerendered = run_job(&app, &mut store, &source, request)?;
    let replacement: Phrase = serde_json::from_value(
        rerendered
            .iter()
            .find(|v| v["kind"] == "phrase")
            .ok_or("missing regenerated voice")?["row"]
            .clone(),
    )?;
    if replacement.id != row.id
        || replacement.text != row.text
        || replacement.seed == old_seed
        || replacement.audio_path == old_audio
    {
        return Err("voice-only regeneration changed text or reused old audio".into());
    }
    let replaced_pcm = kavychka_platform::audio::decode_recording(
        &store
            .audio_file(&replacement)?
            .ok_or("missing replacement recording")?,
        &|| false,
    )?;
    if store.phrases()?.len() != original_count + 1 {
        return Err("generation duplicated or lost library rows".into());
    }
    if store.phrase(&row.id)?.used_at.is_some() {
        return Err("preview consumed a generated phrase".into());
    }
    let after = preview::inventory(&source, &AtomicBool::new(false))?;
    if before != after {
        return Err("original storage changed during model verification".into());
    }
    // Instantiate the same manager over the saved store: origin and settings survive reopening.
    drop(store);
    let mut store = Sandbox::open(&data)?;
    let manager = Manager::new(&mut store, None, Default::default())?;
    if manager.running() {
        return Err("reopening unexpectedly started inference".into());
    }
    drop(manager);
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"original_rows":original_count,"working_rows":store.phrases()?.len(),"imported_rows":origin.imported_rows,"source_fingerprint_before":before.fingerprint,"source_fingerprint_after":after.fingerprint,"models_copied":false,"weights_downloaded":false,"real_text_model":prefs.generation.text_model,"real_voice_model":prefs.generation.tts_model,"generated_pcm_bytes":decoded.len(),"regenerated_pcm_bytes":replaced_pcm.len(),"voice_only_preserved_id_and_text":true,"regeneration_changed_seed_and_audio":true,"preview_did_not_claim":true,"reopened":true,"live_storage_switched":false}),
        )?,
    )?;
    println!(
        "Installed-model/library verification passed: {}",
        evidence.display()
    );
    Ok(())
}
