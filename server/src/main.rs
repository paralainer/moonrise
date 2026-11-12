//! The `moonrise-server` binary: the allocator tuned, a Tokio runtime, and the command line
//! (`moonrise_server::run`, in `cli.rs`).

use anyhow::Result;

/// glibc keeps what is freed for reuse, and with a heap arena per thread that is touched (up to
/// 8 per core) it kept a lot: the SMB block cache's 256 KiB blocks and 1 MiB reads are freed in
/// one thread and asked for again in another, and RSS grew by gigabytes as blocks were dropped and
/// read again (a cancelled read finishes and fills the cache), far past the cache's own cap. Two
/// arenas, and what is free handed back every 10 s, keep it near the live size (measured: about
/// 620 MB under 24,000 cancelled downloads with the default 384 MB cache, 430 MB once quiet,
/// where it had passed 1 GB; no slower). `MALLOC_ARENA_MAX` still wins.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn tune_malloc() {
    if std::env::var_os("MALLOC_ARENA_MAX").is_none() {
        // SAFETY: mallopt only sets an allocator parameter; before any other thread starts.
        unsafe {
            libc::mallopt(libc::M_ARENA_MAX, 2);
        }
    }
    let _ = std::thread::Builder::new().name("malloc-trim".into()).spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        // SAFETY: gives free pages back to the system; nothing else.
        unsafe {
            libc::malloc_trim(0);
        }
    });
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn tune_malloc() {}

fn main() -> Result<()> {
    tune_malloc();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(moonrise_server::run())
}
