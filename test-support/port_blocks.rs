//! Port blocks for tests that start servers on ports PDT picks from a range (runs). Shared by test crates with
//! `#[path = ".../test-support/port_blocks.rs"] mod port_blocks;`.
//!
//! A block is claimed with an OS file lock held until the test process exits, so no two test processes (cargo-nextest
//! runs every test in its own) and no two calls in one process ever get the same block. A random block per process
//! alone was not enough: with a process per test, two tests often drew the same block and one server lost its port.
//!
//! Every range stays below 32768, Linux's first ephemeral port (Windows and macOS start at 49152): the OS never hands
//! those ports to another socket, and each test crate keeps to its own range:
//! - 10000–19999: pdt-api runs (`tests/runs.rs`, blocks of 40)
//! - 20000–29999: pdtd lifecycle runs (`tests/lifecycle.rs`, blocks of 40)
//! - 30000–30249: pdt-host's fake pdtd (`tests/supervise.rs`)
//! - 30500–30999: pdtd's own test daemons (`tests/common`)
//! - 31000–31099: pdt-agents' terminal tests (`tests/pty.rs`, a server a process tree holds)
//! - 31100–31199: ports nothing listens on ([`closed`]), for a server that can't be reached

use std::sync::Mutex;

/// Claims a free block of `size` ports among `blocks` blocks starting at `first`, and returns its first port.
pub fn claim(first: u16, blocks: u16, size: u16) -> u16 {
    static HELD: Mutex<Vec<std::fs::File>> = Mutex::new(Vec::new());
    let dir = std::env::temp_dir().join("pdt-test-port-blocks");
    std::fs::create_dir_all(&dir).expect("the port block lock folder");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    let seed = (std::process::id() ^ nanos) as usize;
    for i in 0..blocks as usize {
        let start = first + ((seed + i) % blocks as usize) as u16 * size;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{start}.lock")))
            .expect("a port block lock file");
        if file.try_lock().is_ok() {
            HELD.lock().unwrap_or_else(|e| e.into_inner()).push(file);
            return start;
        }
    }
    panic!("all {blocks} port blocks from {first} are in use");
}

/// A port no test ever listens on, held for this process: connecting to it is refused. A port bound and then dropped
/// is not: another test process can take it in between.
#[allow(dead_code)]
pub fn closed() -> u16 {
    claim(31100, 100, 1)
}
