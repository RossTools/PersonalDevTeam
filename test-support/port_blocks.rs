//! Port blocks for tests that start servers on ports PDT picks from a range (runs). Shared by test crates with
//! `#[path = ".../test-support/port_blocks.rs"] mod port_blocks;`.
//!
//! A block is claimed with an OS file lock held until the test process exits, so no two test processes (cargo-nextest
//! runs every test in its own) and no two calls in one process ever get the same block. A random block per process
//! alone was not enough: with a process per test, two tests often drew the same block and one server lost its port.

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
