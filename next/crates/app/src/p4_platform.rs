//! Read-only OS qualification and directed messages to this monitor's HWND.
//! These messages test adapter wiring; they never claim a physical lock/sleep.
use kavychka_core::engine::Environment;
use kavychka_platform::{
    audio,
    input::{Monitor, Observation},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&output)?;
    let monitor = Monitor::start()?;
    let registrations = monitor.registrations();
    if !registrations.0 || !registrations.1 {
        return Err(format!("session/power registration unavailable: {registrations:?}").into());
    }
    let initial = kavychka_platform::desktop::environment();
    let mut env = Environment::default();
    monitor.update_environment(&mut env);
    #[cfg(windows)]
    {
        if !monitor.post_session_for_test(7) {
            return Err("cannot send own-window lock message".into());
        }
        monitor.update_environment(&mut env);
        if !env.locked {
            return Err("lock message not applied by registered adapter".into());
        }
        if !monitor
            .drain()
            .iter()
            .any(|o| matches!(o, Observation::SessionChanged(_)))
        {
            return Err("session change did not invalidate input continuity".into());
        }
        monitor.post_session_for_test(8);
        env = Environment::default();
        monitor.update_environment(&mut env);
        if env.locked || env.session_unknown {
            return Err("unlock did not clear adapter state".into());
        }
        monitor.post_session_for_test(2);
        env = Environment::default();
        monitor.update_environment(&mut env);
        if !env.session_unknown {
            return Err("disconnect did not mark uncertainty".into());
        }
        monitor.post_session_for_test(1);
        env = Environment::default();
        monitor.update_environment(&mut env);
        if env.session_unknown {
            return Err("reconnect did not restore a known active session".into());
        }
        monitor.post_power_for_test(4);
        monitor.update_environment(&mut env);
        if !env.suspended {
            return Err("suspend not applied".into());
        }
        std::thread::sleep(Duration::from_millis(35));
        monitor.post_power_for_test(18);
        env = Environment::default();
        monitor.update_environment(&mut env);
        if env.suspended {
            return Err("resume did not clear suspend".into());
        }
        if !monitor
            .drain()
            .iter()
            .any(|o| matches!(o,Observation::Suspension(_,duration) if *duration>=20))
        {
            return Err("paired sleep duration missing".into());
        }
        monitor.post_power_for_test(10);
        if !monitor
            .drain()
            .iter()
            .any(|o| matches!(o, Observation::PowerChanged(_)))
        {
            return Err("power change not forwarded".into());
        }
    }
    let shutdown = Instant::now();
    drop(monitor);
    if shutdown.elapsed() > Duration::from_secs(2) {
        return Err("monitor shutdown too slow".into());
    }
    // Validate real WASAPI lifecycle separately from the directed OS messages.
    let (sender, receiver) = std::sync::mpsc::channel();
    let native = audio::Output::with_phase_observer(Some(std::sync::Arc::new(move |phase| {
        let _ = sender.send(phase);
    })));
    native.play(
        "",
        kavychka_core::preferences::Language::En,
        kavychka_core::preferences::AudioMode::Chime,
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut started = false;
    let mut completed = false;
    while Instant::now() < deadline && !completed {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(audio::Phase::Started) => started = true,
            Ok(audio::Phase::Completed) => completed = true,
            Ok(audio::Phase::Failed) => return Err("native audio probe failed".into()),
            _ => {}
        }
    }
    if !started || !completed {
        return Err("WASAPI lifecycle incomplete".into());
    }
    drop(native);
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"wts_registered":registrations.0,"power_registered":registrations.1,
            "directed_own_window_lock_unlock_disconnect_suspend_resume_power":true,"session_input_continuity_reset":true,
            "native_wasapi_started_completed":true,"monitor_joined":true,
            "observed_initial_locked":initial.locked,"observed_initial_fullscreen":initial.fullscreen,
            "notification_signal":format!("{:?}",initial.notifications),"power_source":format!("{:?}",initial.power),
            "exact_dnd_switch_supported":false,"physical_session_sleep_or_power_transition_tested":false,
            "physical_device_disconnect_tested":false,"desktop_input_injected":false,"user_library_opened":false
        }))?,
    )?;
    println!("P4 platform registration/own-window messages/native audio lifecycle passed.");
    Ok(())
}
