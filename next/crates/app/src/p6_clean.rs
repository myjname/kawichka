//! Fresh CPU text environment, reusing explicit installed GGUF weights read-only.
use crate::inference;
use kavychka_storage::sandbox::Sandbox;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub fn run(output: PathBuf, descriptor: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !output.is_absolute() || output.exists() {
        return Err("New absolute CPU qualification folder required".into());
    }
    let descriptor: Value = serde_json::from_slice(&std::fs::read(descriptor)?)?;
    let mut prefs: kavychka_core::preferences::Preferences =
        serde_json::from_value(descriptor["preferences"].clone())?;
    prefs.generation.device = "cpu".into();
    prefs.generation.tts_model = "system".into();
    prefs.generation.voice_profile.clear();
    prefs.generation.text_enabled = true;
    prefs.generation.auto_daily = false;
    prefs.desktop.automatic_updates = false;
    prefs.generation.character_description =
        kavychka_core::generation::Generation::default().character_description;
    prefs.user_name.clear();
    let app = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let id = prefs.generation.text_model.clone();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(app.join("assets/components.json"))?)?;
    if manifest["models"][&id]["format"] != "gguf" {
        return Err("Choose an installed GGUF model for the CPU environment check".into());
    }
    std::fs::create_dir_all(&output)?;
    let mut store = Sandbox::open(&output.join("data"))?;
    store.save(&prefs)?;
    let resources = output.join("empty-resources");
    std::fs::create_dir(&resources)?;
    let day = chrono::Utc::now().date_naive().to_string();
    let mut request = inference::request(&prefs, &[], "install", &day);
    request["ids"] = json!([id]);
    let installed = crate::models_smoke::run_job(&app, &mut store, &resources, request)?;
    if !installed.iter().any(|v| {
        v["kind"] == "result" && v["downloaded_models"].as_array().is_some_and(Vec::is_empty)
    }) {
        return Err("CPU check unexpectedly requested model weights".into());
    }
    let mut request = inference::request(&prefs, &[], "generate", &day);
    request["tasks"] = json!([{"category":"break","followup":false}]);
    request["approved"] = json!(true);
    let generated = crate::models_smoke::run_job(&app, &mut store, &resources, request)?;
    if !generated
        .iter()
        .any(|v| v["kind"] == "phrase" && v["row"]["model"] == "system")
    {
        return Err("Fresh CPU inference did not produce a phrase".into());
    }
    if store.root.join("components/models").exists() {
        return Err("Fresh CPU test created a model weight directory".into());
    }
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"fresh_python_and_cpu_llama":true,"device":"cpu","text_model":prefs.generation.text_model,"voice_model":"system","torch_voice_environment_tested":false,"cuda_environment_tested":false,"weights_downloaded":false,"real_inference":true,"live_user_storage_changed":false}),
        )?,
    )?;
    Ok(())
}
