//! One owned inference process at a time. JSON messages, no shared SQLite writer.
use kavychka_core::preferences::Preferences;
use kavychka_storage::library::Phrase;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct PendingStaging {
    root: PathBuf,
    armed: bool,
}
impl Drop for PendingStaging {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        fn validate(path: &Path, depth: usize, count: &mut usize) -> bool {
            *count += 1;
            if depth > 64
                || *count > 20_000
                || kavychka_storage::preview::no_link_chain(path).is_err()
            {
                return false;
            }
            if path.is_dir() {
                let Ok(entries) = std::fs::read_dir(path) else {
                    return false;
                };
                for entry in entries {
                    let Ok(entry) = entry else { return false };
                    if !validate(&entry.path(), depth + 1, count) {
                        return false;
                    }
                }
            }
            true
        }
        let Some(parent) = self.root.parent() else {
            return;
        };
        if kavychka_storage::preview::no_link_chain(parent).is_err() {
            return;
        }
        let (Ok(root), Ok(base)) = (self.root.canonicalize(), parent.canonicalize()) else {
            return;
        };
        if root.parent() != Some(base.as_path())
            || !root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("job-"))
        {
            return;
        }
        if validate(&root, 0, &mut 0) {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
fn runtime_ready(root: &Path, device: &str, profile: Option<&str>) -> bool {
    if !root.join("runtime/python.exe").is_file() {
        return false;
    }
    let marker: Value = std::fs::read(root.join("runtime-ok.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    marker.is_object()
        && profile.is_none_or(|profile| marker["profile"] == profile)
        && (device != "cuda"
            || marker["gpu"] == true
            || marker["torch"].as_str().is_some_and(|s| s.contains("+cu")))
}
fn cosy_ready(root: &Path, revision: Option<&str>, profile: Option<&str>) -> bool {
    let marker: Value = std::fs::read(root.join("cosy/verified.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    marker.is_object()
        && revision.is_none_or(|revision| marker["revision"] == revision)
        && profile.is_none_or(|profile| marker["profile"] == profile)
        && root.join("cosy/site/numpy/__init__.py").is_file()
        && root.join("cosy/Whisper/whisper/__init__.py").is_file()
        && root
            .join("cosy/CosyVoice/cosyvoice/cli/cosyvoice.py")
            .is_file()
        && root.join("cosy/Matcha-TTS/matcha/utils/audio.py").is_file()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub installed: bool,
    pub environment: bool,
    pub bytes: u64,
    pub path: PathBuf,
}
pub fn models(
    app: &Path,
    resources: &Path,
    library: &Path,
    prefs: &Preferences,
) -> Result<Vec<Model>, String> {
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(app.join("assets/components.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut list = vec![Model {
        id: "system".into(),
        name: "Windows · System voice".into(),
        kind: "voice".into(),
        installed: true,
        environment: true,
        bytes: 0,
        path: PathBuf::new(),
    }];
    for (id, spec) in manifest["models"]
        .as_object()
        .ok_or("invalid component manifest")?
    {
        let managed = library.join("components/models").join(id);
        let managed_ready = spec["files"].as_array().is_some_and(|files| {
            files.iter().all(|f| {
                managed
                    .join(f["path"].as_str().unwrap_or(""))
                    .metadata()
                    .is_ok_and(|m| m.is_file() && Some(m.len()) == f["size"].as_u64())
            })
        });
        let path = if managed_ready {
            managed
        } else {
            prefs
                .generation
                .model_paths
                .get(id)
                .map(PathBuf::from)
                .unwrap_or_else(|| resources.join("models").join(id))
        };
        let mut installed = true;
        let mut bytes = 0;
        for f in spec["files"].as_array().ok_or("invalid model files")? {
            let name = f["path"].as_str().ok_or("invalid model file")?;
            let size = f["size"].as_u64().ok_or("invalid model size")?;
            bytes += size;
            installed &= path
                .join(name)
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() == size);
        }
        let managed_root = library.join("components");
        let runtime_roots = [resources, managed_root.as_path()];
        let python = runtime_roots
            .iter()
            .any(|root| root.join("runtime/python.exe").is_file());
        let environment = python
            && runtime_roots
                .iter()
                .any(|root| match spec["format"].as_str() {
                    Some("gguf") => {
                        root.join("llama/llama-server.exe").is_file()
                            && root.join("llama/verified.json").is_file()
                    }
                    Some("cosyvoice3") => {
                        runtime_roots.iter().any(|r| {
                            runtime_ready(
                                r,
                                &prefs.generation.device,
                                manifest["runtime_profile"].as_str(),
                            )
                        }) && cosy_ready(
                            root,
                            manifest["cosy_runtime"]["revision"].as_str(),
                            manifest["runtime_profile"].as_str(),
                        )
                    }
                    _ => runtime_ready(
                        root,
                        &prefs.generation.device,
                        manifest["runtime_profile"].as_str(),
                    ),
                });
        list.push(Model {
            id: id.clone(),
            name: spec["name"]
                .as_str()
                .unwrap_or(match id.as_str() {
                    "tts-0.6b" => "Qwen3-TTS 0.6B",
                    "tts-1.7b" => "Qwen3-TTS 1.7B",
                    "text-1.7b" => "Qwen3 1.7B · legacy Transformers",
                    _ => id,
                })
                .into(),
            kind: if id.starts_with("text-") {
                "text"
            } else {
                "voice"
            }
            .into(),
            installed,
            environment,
            bytes,
            path,
        });
    }
    Ok(list)
}
pub struct Worker {
    pub root: PathBuf,
    pub cancelled: Arc<AtomicBool>,
    child: Child,
    job: Option<kavychka_platform::process::Job>,
    reader: Option<std::thread::JoinHandle<()>>,
    messages: mpsc::Receiver<Result<Value, String>>,
    started: Instant,
    finished: bool,
    cancelling: Option<Instant>,
}
impl Worker {
    pub fn start(
        app: &Path,
        library: &Path,
        resources: &Path,
        mut request: Value,
    ) -> Result<Self, String> {
        let id = format!(
            "job-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let root = library.join("jobs").join(format!(
            "{id}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
        std::fs::create_dir(&root).map_err(|e| e.to_string())?;
        let mut pending = PendingStaging {
            root: root.clone(),
            armed: true,
        };
        let mut payload = serde_json::Map::new();
        for key in ["known_texts", "rows"] {
            if let Some(value) = request.as_object_mut().and_then(|m| m.remove(key)) {
                payload.insert(key.into(), value);
            }
        }
        if !payload.is_empty() {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("payload.json"))
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(file, &payload).map_err(|e| e.to_string())?;
            if std::fs::metadata(root.join("payload.json"))
                .map_err(|e| e.to_string())?
                .len()
                > 64_000_000
            {
                return Err("Library snapshot exceeds the 64 MB task limit".into());
            }
            request["payload_file"] = json!("payload.json");
        }
        let managed_python = library.join("components/runtime/python.exe");
        let original_python = resources.join("runtime/python.exe");
        let installing = request["operation"] == "install";
        let repairing = installing && request["repair_environment"] == true;
        let voices = request["ids"].as_array().is_some_and(|ids| {
            ids.iter().any(|i| {
                i.as_str()
                    .is_some_and(|s| s.starts_with("tts-") || s == "text-1.7b")
            })
        });
        let cosy = request["ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|i| i == "tts-cosyvoice3"));
        let device = request["settings"]["device"].as_str().unwrap_or("auto");
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(app.join("assets/components.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let profile = manifest["runtime_profile"].as_str();
        let managed_full = runtime_ready(&library.join("components"), device, profile);
        let original_full = runtime_ready(resources, device, profile);
        let needs_bootstrap = installing
            && (repairing
                || !managed_python.is_file() && !original_python.is_file()
                || voices && !managed_full && !original_full
                || cosy
                    && !cosy_ready(
                        &library.join("components"),
                        manifest["cosy_runtime"]["revision"].as_str(),
                        manifest["runtime_profile"].as_str(),
                    )
                    && !cosy_ready(
                        resources,
                        manifest["cosy_runtime"]["revision"].as_str(),
                        manifest["runtime_profile"].as_str(),
                    ));
        let python = if managed_full
            || !original_python.is_file()
                && library
                    .join("components/runtime/python-bootstrap.json")
                    .is_file()
        {
            managed_python
        } else {
            original_python
        };
        if !python.is_file() && !needs_bootstrap {
            return Err(
                "Совместимая среда Python не найдена. Системная озвучка и готовые записи доступны."
                    .into(),
            );
        }
        request["version"] = json!(1);
        request["task"] = json!(id);
        request["work_dir"] = json!(root);
        request["library_root"] = json!(library);
        request["components_root"] = json!(if repairing {
            library.join("components")
        } else {
            resources.to_owned()
        });
        let log = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("worker.log"))
            .map_err(|e| e.to_string())?;
        let mut command = if needs_bootstrap {
            let mut cmd =
                std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            cmd.arg("--bootstrap-worker");
            cmd
        } else {
            let mut cmd = std::process::Command::new(python);
            cmd.arg("-B").arg(app.join("next/worker.py"));
            cmd
        };
        command
            .current_dir(app)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let job = match kavychka_platform::process::Job::assign_background(&child) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let send = (|| -> Result<(), String> {
            let mut input = child.stdin.take().ok_or("worker stdin missing")?;
            let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
            if bytes.len() > 4_000_000 {
                return Err("worker request too large".into());
            }
            input
                .write_all(&bytes)
                .and_then(|_| input.write_all(b"\n"))
                .map_err(|e| e.to_string())
        })();
        if let Err(e) = send {
            drop(job);
            let _ = child.wait();
            return Err(e);
        }
        let stdout = child.stdout.take().ok_or("worker stdout missing")?;
        let (tx, messages) = mpsc::channel();
        let expected = id.clone();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                use std::io::Read;
                let result = reader
                    .by_ref()
                    .take(1_000_001)
                    .read_until(b'\n', &mut bytes);
                match result {
                    Ok(0) => break,
                    Ok(_) if bytes.len() > 1_000_000 => {
                        let _ = tx.send(Err("worker message exceeds limit".into()));
                        break;
                    }
                    Ok(_) => {
                        let value = serde_json::from_slice::<Value>(&bytes)
                            .map_err(|e| e.to_string())
                            .and_then(|v| {
                                if v["version"] == 1 && v["task"] == expected {
                                    Ok(v)
                                } else {
                                    Err("worker protocol/task mismatch".into())
                                }
                            });
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });
        let worker = Self {
            root,
            cancelled: Arc::new(AtomicBool::new(false)),
            child,
            job: Some(job),
            reader: Some(reader),
            messages,
            started: Instant::now(),
            finished: false,
            cancelling: None,
        };
        pending.armed = false;
        Ok(worker)
    }
    pub fn request_cancel(&mut self) {
        if self.cancelling.is_none() {
            self.cancelling = Some(Instant::now());
            if std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.root.join("cancel.request"))
                .and_then(|mut file| file.write_all(b"cancel\n"))
                .is_err()
            {
                self.terminate();
            }
        }
    }
    pub fn cancelling(&self) -> bool {
        self.cancelling.is_some()
    }
    pub fn poll(&mut self) -> Vec<Result<Value, String>> {
        let mut output: Vec<_> = self.messages.try_iter().collect();
        if self.cancelled.load(Ordering::Acquire) {
            self.request_cancel();
        }
        if self
            .cancelling
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(3))
            || self.started.elapsed() > Duration::from_secs(1800)
        {
            self.terminate();
            output.clear();
            output.push(Err("Генерация отменена или превышено время ожидания".into()));
            return output;
        }
        if self.cancelling.is_some() {
            output.clear();
            if self.finished || self.child.try_wait().ok().flatten().is_some() {
                self.terminate();
                output.push(Err("Генерация отменена / Generation cancelled".into()));
            }
            return output;
        }
        if output.iter().any(|v| {
            v.as_ref()
                .is_ok_and(|v| v["kind"] == "result" || v["kind"] == "error")
        }) {
            self.finished = true;
        }
        if !self.finished && self.child.try_wait().ok().flatten().is_some() {
            // Reader may still be handing over the last line; join before draining.
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
            output.extend(self.messages.try_iter());
            if !output.iter().any(|v| {
                v.as_ref()
                    .is_ok_and(|v| v["kind"] == "result" || v["kind"] == "error")
            }) {
                output.push(Err(format!(
                    "Генератор завершился без результата. Журнал: {}",
                    self.root.join("worker.log").display()
                )));
            }
            self.finished = true;
        }
        output
    }
    pub fn done(&self) -> bool {
        self.finished
    }
    fn terminate(&mut self) {
        drop(self.job.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.finished = true;
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}
pub fn request(prefs: &Preferences, rows: &[Phrase], operation: &str, day: &str) -> Value {
    let language = if prefs.speech_language == kavychka_core::preferences::Language::Ru {
        "ru"
    } else {
        "en"
    };
    json!({"operation":operation,"settings":prefs.generation.worker_settings(language,&prefs.user_name),"known_texts":rows.iter().map(|r|r.text.clone()).collect::<Vec<_>>(),"history":rows.iter().take(500).map(|r|r.text.clone()).collect::<Vec<_>>(),"styles":rows.iter().filter(|r|!r.voice_style.is_empty()).take(6).map(|r|r.voice_style.clone()).collect::<Vec<_>>(),"day":day,"created_at":chrono::Utc::now().to_rfc3339(),"rotation":rows.iter().filter(|r|!r.voice_reference.is_empty()).count(),"approved":prefs.generation.daily_auto_keep})
}
#[cfg(test)]
mod tests {
    #[test]
    fn old_runtime_profile_requires_explicit_upgrade_and_does_not_edit_its_marker() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../work/runtime-profile-tests");
        std::fs::create_dir_all(&base).unwrap();
        let t = tempfile::tempdir_in(base).unwrap();
        std::fs::create_dir(t.path().join("runtime")).unwrap();
        std::fs::write(t.path().join("runtime/python.exe"), b"fixture").unwrap();
        let marker = t.path().join("runtime-ok.json");
        let old = br#"{"torch":"2.9.1+cu128","gpu":true}"#;
        std::fs::write(&marker, old).unwrap();
        assert!(!super::runtime_ready(t.path(), "cuda", Some("new-profile")));
        assert_eq!(std::fs::read(&marker).unwrap(), old);
        std::fs::write(
            &marker,
            br#"{"torch":"2.14.1+cu126","gpu":true,"profile":"new-profile"}"#,
        )
        .unwrap();
        assert!(super::runtime_ready(t.path(), "cpu", Some("new-profile")));
        assert!(super::runtime_ready(t.path(), "cuda", Some("new-profile")));
    }

    #[test]
    fn failed_worker_start_removes_its_private_payload_without_touching_the_library() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../work/failed-worker-tests");
        std::fs::create_dir_all(&base).unwrap();
        let t = tempfile::tempdir_in(base).unwrap();
        let store = kavychka_storage::sandbox::Sandbox::open(&t.path().join("data")).unwrap();
        let prefs = store.load().unwrap();
        let before = kavychka_maintenance::digest(&store.root.join("shell.sqlite3")).unwrap();
        for _ in 0..3 {
            let mut request = super::request(&prefs, &[], "generate", "2026-10-24");
            request["known_texts"] = serde_json::json!(["Private staging fixture text"]);
            assert!(
                super::Worker::start(
                    t.path(),
                    &store.root,
                    &t.path().join("absent-runtime"),
                    request
                )
                .is_err()
            );
        }
        assert_eq!(
            std::fs::read_dir(store.root.join("jobs")).unwrap().count(),
            0
        );
        assert_eq!(
            before,
            kavychka_maintenance::digest(&store.root.join("shell.sqlite3")).unwrap()
        );
    }
}
