//! LLVM's target registry and `libLTO`'s global state take no lock of their own, so Lyftar must keep safe code on several
//! threads from racing on them. One such race ties the registry's list of targets into a loop, and every later lookup spins
//! for good, so each try is a new process, where the first use really is the first, with a time limit.

use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const TRIES: u32 = 40;
const TIME_LIMIT: Duration = Duration::from_secs(20);

fn race_on_first_use() {
    let first_uses: [fn(); 3] = [
        || {
            // Junk comes back as an error, after `libLTO` has set itself up.
            assert!(lyftar::lto::LtoModule::from_memory(b"not bitcode").is_err());
            let _ = lyftar::lto::LtoModule::in_local_context(b"not bitcode either", "second").is_err();
        },
        || {
            lyftar::target::initialize_native_target().expect("this machine's target");
            lyftar::jit::Jit::new().expect("a JIT");
        },
        || {
            lyftar::target::initialize_all_targets();
            lyftar::target::initialize_target("AArch64").expect("this target");
            assert!(lyftar::target::initialize_target("NoSuchTarget").is_err());
            lyftar::target::TargetMachine::for_host().expect("this machine");
        },
    ];
    let barrier = Arc::new(Barrier::new(first_uses.len()));
    let threads: Vec<_> = first_uses
        .into_iter()
        .map(|first_use| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                first_use();
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a thread that doesn't panic");
    }
}

#[test]
fn decision_16_first_uses_on_several_threads_do_not_race() {
    if std::env::var_os("LYFTAR_RACE_CHILD").is_some() {
        race_on_first_use();
        return;
    }
    for attempt in 0..TRIES {
        let mut child = Command::new(std::env::current_exe().expect("this test's path"))
            .args(["--exact", "decision_16_first_uses_on_several_threads_do_not_race", "--nocapture"])
            .env("LYFTAR_RACE_CHILD", "1")
            .stdout(Stdio::null())
            .spawn()
            .expect("running the child");
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("waiting for the child") {
                break status;
            }
            if start.elapsed() > TIME_LIMIT {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "try {attempt} was still running after {TIME_LIMIT:?}: threads are spinning on LLVM's global state"
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(status.success(), "try {attempt} ended with {status}");
    }
}
