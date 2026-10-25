//! Real OS audio/input lifecycle checks, separate from controlled UI scenarios.
use kavychka_core::preferences::{AudioMode, Language};
use kavychka_platform::{audio, input::Monitor};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&output)?;
    let monitor = Monitor::start()?;
    let input_events = monitor.drain().len();
    let stopped = Instant::now();
    drop(monitor);
    if stopped.elapsed() > Duration::from_secs(2) {
        return Err("input monitor shutdown exceeded 2 seconds".into());
    }
    let english = audio::synthesis_probe(Language::En);
    let russian = audio::synthesis_probe(Language::Ru);
    if english.as_ref().is_err() && russian.as_ref().is_err() {
        return Err(format!(
            "no installed system voice could be verified: en={english:?}; ru={russian:?}"
        )
        .into());
    }
    audio::hardware_probe()?;
    let devices = audio::devices()?;
    let endpoint = devices
        .iter()
        .find(|d| d.is_default)
        .ok_or("no default endpoint enumerated")?;
    let pinned = audio::Output::new();
    pinned.play_on("", Language::En, AudioMode::Chime, &endpoint.id)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let feedback = loop {
        if let Some(result) = pinned.feedback() {
            break result;
        }
        if Instant::now() > deadline {
            return Err("pinned output test timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    feedback?;
    while pinned.busy() {
        std::thread::sleep(Duration::from_millis(2));
    }
    pinned.play_on(
        "",
        Language::En,
        AudioMode::Chime,
        "p3-nonexistent-endpoint",
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = pinned.feedback() {
            if result.is_ok() {
                return Err("missing pinned output silently fell back".into());
            }
            break;
        }
        if Instant::now() > deadline {
            return Err("missing output test timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(pinned);
    let output_audio = audio::Output::new();
    output_audio.play(
        "A short test of cancellation. A short test of cancellation.",
        Language::En,
        AudioMode::All,
    )?;
    std::thread::sleep(Duration::from_millis(25));
    let start = Instant::now();
    output_audio.stop();
    while output_audio.busy() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    if output_audio.busy() {
        return Err("audio cancellation exceeded 2 seconds".into());
    }
    let cancel_ms = start.elapsed().as_millis();
    drop(output_audio);
    let voice = |result: Result<usize, String>| match result {
        Ok(bytes) => serde_json::json!({"available":true,"pcm_bytes":bytes}),
        Err(error) => serde_json::json!({"available":false,"reason":error}),
    };
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":true,"raw_input_registered_and_joined":true,"input_counts_only":input_events,"system_voice_en":voice(english),"system_voice_ru":voice(russian),"wasapi_chime_played":true,"pinned_endpoint_played":true,"missing_pin_no_fallback":true,"enumerated_device_count":devices.len(),"cancellation_ms":cancel_ms,"audio_thread_joined":true,"physical_output_heard_by_human":false,"physical_endpoint_disconnect_tested":false}),
        )?,
    )?;
    println!("Native input/system-speech/WASAPI/cancellation probes passed.");
    Ok(())
}
