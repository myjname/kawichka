//! Opt-in benchmark instrumentation. The watchdog reads only our UI thread and
//! process; no native UI, thread suspension, global trace, or live-data access.
use kavychka_platform::process::ThreadObserver;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
struct State {
    phase: AtomicU64,
    since: AtomicU64,
    processor: AtomicU64,
    stop: AtomicBool,
    samples: Mutex<Vec<Value>>,
}
pub struct Probe {
    state: Arc<State>,
    clock: Instant,
    sequence: u64,
    thread: ThreadObserver,
    watcher: Option<std::thread::JoinHandle<()>>,
    output: PathBuf,
}
impl Probe {
    pub fn new(clock: Instant, output: PathBuf) -> Result<Self, String> {
        let thread = ThreadObserver::current()?;
        let watched = ThreadObserver::current()?;
        let ui_thread = watched.id();
        let stack_hints = std::env::var_os("KAVYCHKA_TRACE_STACK_HINTS").is_some_and(|v| v == "1");
        let state = Arc::new(State {
            phase: AtomicU64::new(0),
            since: AtomicU64::new(0),
            processor: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            samples: Mutex::new(vec![]),
        });
        let watch = state.clone();
        let watcher = std::thread::spawn(move || {
            let mut previous = clock.elapsed();
            let mut hinted = 0;
            while !watch.stop.load(Ordering::Acquire) {
                let now = clock.elapsed();
                let gap = now.saturating_sub(previous);
                previous = now;
                let phase = watch.phase.load(Ordering::Acquire);
                let since = watch.since.load(Ordering::Acquire);
                let age = now.saturating_sub(Duration::from_nanos(since));
                if stack_hints && phase != 0 && phase != hinted && age > Duration::from_millis(60) {
                    hinted = phase;
                    eprintln!(
                        "KAVYCHKA_UI_STALL {}",
                        json!({"pid":std::process::id(),"thread":ui_thread,"phase":phase & 3,"sequence":phase >> 2,"elapsed_seconds":now.as_secs_f64()})
                    );
                }
                if gap > Duration::from_millis(50) || phase != 0 && age > Duration::from_millis(50)
                {
                    let thread = watched.sample();
                    let process = kavychka_platform::process::sample();
                    if watch.phase.load(Ordering::Acquire) == phase {
                        let record = match (thread, process) {
                            (Ok((cpu, io)), Ok(p)) => json!({"elapsed_seconds":now.as_secs_f64(),
                                "phase":phase & 3,"sequence":phase >> 2,
                                "phase_age_ms":age.as_secs_f64()*1000.,"watchdog_gap_ms":gap.as_secs_f64()*1000.,
                                "ui_processor_at_phase_start":watch.processor.load(Ordering::Acquire).checked_sub(1),
                                "ui_cpu_100ns":cpu,"ui_pending_io":io,"page_faults":p.page_faults,
                                "process_read_bytes":p.read_bytes,"available_physical_bytes":p.available_physical_bytes}),
                            (thread, process) => {
                                json!({"error":format!("Thread/process sample: {:?}/{:?}",thread.err(),process.err())})
                            }
                        };
                        watch.samples.lock().unwrap().push(record);
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        Ok(Self {
            state,
            clock,
            sequence: 0,
            thread,
            watcher: Some(watcher),
            output,
        })
    }
    pub fn begin(&mut self, phase: u64) {
        self.sequence += 1;
        self.state.processor.store(
            kavychka_platform::ui_scheduling::current_processor().map_or(0, |n| n as u64 + 1),
            Ordering::Release,
        );
        self.state
            .since
            .store(self.clock.elapsed().as_nanos() as u64, Ordering::Release);
        self.state
            .phase
            .store((self.sequence << 2) | phase, Ordering::Release);
    }
    pub fn end(&self) {
        self.state.phase.store(0, Ordering::Release);
    }
    pub fn cpu(&self) -> Result<u64, String> {
        self.thread.sample().map(|v| v.0)
    }
    pub fn samples(&self) -> Vec<Value> {
        self.state.samples.lock().unwrap().clone()
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.samples()) {
            let _ = std::fs::write(&self.output, bytes);
        }
    }
}
