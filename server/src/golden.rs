//! Golden files for characterisation tests: [`check`] compares what a test produced with
//! `testdata/golden/<name>.txt`, so a refactor that changes an output fails. To write (or
//! rewrite) them, run the tests with `UPDATE_GOLDEN=1` and review the diff:
//!
//! ```text
//! scripts/rust-dev.sh env UPDATE_GOLDEN=1 cargo test --locked golden
//! ```

use std::path::PathBuf;

/// Compares `actual` with the golden file `name` (or writes it, under `UPDATE_GOLDEN=1`).
pub fn check(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (UPDATE_GOLDEN=1 writes it)", path.display()));
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{name}: differs from {} at line {}\n--- expected\n{}\n--- actual\n{}",
            path.display(),
            line + 1,
            expected.lines().nth(line).unwrap_or("<end>"),
            actual.lines().nth(line).unwrap_or("<end>"),
        );
    }
}
