//! Explicit bounded CPU-contention fixture; not shipped in the installed app.
//! Parent pins this to ONE logical CPU before the exact stdin handshake.
use std::{
    io::{Read, Write},
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};
fn main() {
    let threads = 48;
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut children = vec![];
    for _ in 0..threads {
        let gate = barrier.clone();
        children.push(std::thread::spawn(move || {
            gate.wait();
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut value = 1u64;
            while Instant::now() < deadline {
                for _ in 0..10000 {
                    value =
                        std::hint::black_box(value.wrapping_mul(1664525).wrapping_add(1013904223));
                }
            }
            value
        }));
    }
    println!("READY");
    std::io::stdout().flush().unwrap();
    let mut token = [0; 3];
    if std::io::stdin().read_exact(&mut token).is_err() || &token != b"run" {
        std::process::exit(2);
    }
    barrier.wait();
    for child in children {
        let _ = child.join();
    }
}
