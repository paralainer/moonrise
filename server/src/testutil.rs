//! Helpers shared by the tests.

use std::path::Path;

/// Writes `script` as the executable `path`, for a test's stand-in ffmpeg or ffprobe.
///
/// Another process writes the executable (`install`). If the test process did, a child started at
/// that moment by a test on another thread would inherit the file open for writing, and until that
/// child's own exec, running the file fails with "Text file busy".
pub fn write_executable(path: &Path, script: &str) {
    let source = path.with_extension("script");
    std::fs::write(&source, script).unwrap();
    let status = std::process::Command::new("install")
        .args(["-m", "755"])
        .arg(&source)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "install: {status}");
}

/// A folder of a test's own under the system temp folder, removed when dropped.
///
/// Its name is unique to the process and the call, so tests running at once, in this process or
/// in another sharing the temp folder (two checkouts, two containers), never share one.
pub struct TempDir(std::path::PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("moonrise-{name}-{}-{nanos}-{n}", std::process::id()));
            // Taken (the same pid in another container sharing the folder): the next name.
            match std::fs::create_dir(&path) {
                Ok(()) => return TempDir(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{}: {e}", path.display()),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
