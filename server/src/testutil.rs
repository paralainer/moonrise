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
