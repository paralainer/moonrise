//! The server's recent log lines, kept in memory for the apps' About and Activity screens
//! (`GET /v1/admin/logs`, the last errors in `GET /v1/admin/activity`). Every line written to
//! stderr is also appended here (the last [`MAX_LINES`]); lines are redacted when read: the
//! secrets the server knows (keys, passwords, the admin token) and anything shaped like a token,
//! key or password are replaced with `[redacted]`.
//!
//! Before anything is written (stderr, the log file, the copy here), [`scrub`] takes credentials
//! out of URLs: the secret of the SMB loopback URLs ffmpeg is handed (`/f/{secret}/…`,
//! `/c/{secret}/…`, see `vfs.rs`), `user:password@` and secret-named query parameters. ffmpeg
//! prints its input URL when it can't open it, and its command lines are logged at debug.
//!
//! `MOONRISE_LOG_FILE` (macOS LaunchAgent; unset in containers): the lines go to that file instead of
//! stderr, which is renamed to `<file>.1` when it passes 10 MB (one old file kept).

use std::{
    collections::VecDeque,
    io::Write,
    sync::{Mutex, OnceLock},
};

pub const MAX_LINES: usize = 2000;

struct Buf {
    lines: VecDeque<String>,
    /// The last lines at WARN or ERROR (kept longer than the general tail).
    problems: VecDeque<String>,
}

fn buf() -> &'static Mutex<Buf> {
    static B: OnceLock<Mutex<Buf>> = OnceLock::new();
    B.get_or_init(|| {
        Mutex::new(Buf {
            lines: VecDeque::new(),
            problems: VecDeque::new(),
        })
    })
}

fn push(line: &str) {
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }
    // Lines are capped (an ffmpeg command line can be long).
    let line: String = line.chars().take(2000).collect();
    let problem = level_of(&line).is_some_and(|l| l == "WARN" || l == "ERROR");
    let mut b = buf().lock().unwrap();
    if b.lines.len() >= MAX_LINES {
        b.lines.pop_front();
    }
    if problem {
        if b.problems.len() >= 200 {
            b.problems.pop_front();
        }
        b.problems.push_back(line.clone());
    }
    b.lines.push_back(line);
}

/// The level word of a formatted line ("2026-11-05T… WARN moonrise_server…").
pub fn level_of(line: &str) -> Option<&'static str> {
    let head: String = line.chars().take(60).collect();
    ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"]
        .into_iter()
        .find(|l| head.split_whitespace().take(3).any(|w| w == *l))
}

const LOG_FILE_MAX: u64 = 10 << 20;

struct LogFile {
    path: std::path::PathBuf,
    file: std::fs::File,
    size: u64,
}

impl LogFile {
    fn open(path: std::path::PathBuf) -> Option<LogFile> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let size = file.metadata().map_or(0, |m| m.len());
        Some(LogFile { path, file, size })
    }

    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.size >= LOG_FILE_MAX {
            let mut old = self.path.clone().into_os_string();
            old.push(".1");
            let _ = std::fs::rename(&self.path, &old);
            if let Some(f) = LogFile::open(self.path.clone()) {
                *self = f;
            }
        }
        let n = self.file.write(data)?;
        self.size += n as u64;
        Ok(n)
    }
}

/// The log file (`MOONRISE_LOG_FILE`), if one is set and could be opened.
fn log_file() -> Option<&'static Mutex<LogFile>> {
    static F: OnceLock<Option<Mutex<LogFile>>> = OnceLock::new();
    F.get_or_init(|| {
        std::env::var_os("MOONRISE_LOG_FILE")
            .filter(|p| !p.is_empty())
            .and_then(|p| LogFile::open(p.into()))
            .map(Mutex::new)
    })
    .as_ref()
}

/// A writer for `tracing_subscriber::fmt`: stderr (or `MOONRISE_LOG_FILE`), and a copy of each line
/// here.
#[derive(Default)]
pub struct Tee {
    pending: Vec<u8>,
}

impl Tee {
    /// One whole line, scrubbed, out to stderr (or the log file) and into the buffer.
    fn emit(line: &[u8]) -> std::io::Result<()> {
        let line = scrub(&String::from_utf8_lossy(line)).into_owned();
        match log_file() {
            Some(f) => {
                let mut f = f.lock().unwrap();
                let mut rest = line.as_bytes();
                while !rest.is_empty() {
                    let n = f.write(rest)?;
                    if n == 0 {
                        return Err(std::io::ErrorKind::WriteZero.into());
                    }
                    rest = &rest[n..];
                }
            }
            None => std::io::stderr().write_all(line.as_bytes())?,
        }
        push(&line);
        Ok(())
    }
}

impl Write for Tee {
    /// Lines go out whole (scrubbing needs all of a URL); a last one without a newline when
    /// dropped. The subscriber makes a `Tee` for each event and writes it in one go.
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(data);
        while let Some(i) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=i).collect();
            Tee::emit(&line)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match log_file() {
            Some(f) => f.lock().unwrap().file.flush(),
            None => std::io::stderr().flush(),
        }
    }
}

impl Drop for Tee {
    fn drop(&mut self) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            let _ = Tee::emit(&line);
        }
    }
}

/// `MakeWriter` for the subscriber.
pub fn make() -> Tee {
    Tee::default()
}

/// The last `n` lines (at `min_level` or above: "warn" keeps WARN and ERROR), oldest first,
/// redacted.
pub fn tail(n: usize, min_level: Option<&str>, secrets: &[String]) -> Vec<String> {
    let b = buf().lock().unwrap();
    let keep = |l: &String| match min_level {
        Some("warn") => level_of(l).is_some_and(|x| x == "WARN" || x == "ERROR"),
        Some("error") => level_of(l) == Some("ERROR"),
        _ => true,
    };
    let src: Vec<&String> = if min_level == Some("warn") {
        b.problems.iter().collect()
    } else {
        b.lines.iter().filter(|l| keep(l)).collect()
    };
    let start = src.len().saturating_sub(n);
    src[start..].iter().map(|l| redact(l, secrets)).collect()
}

/// Credentials in URLs out of text bound for the log (or an error a client sees): the SMB
/// loopback secret (`http://127.0.0.1:…/f/{secret}/…`, `/c/{secret}/…`, and the value itself
/// wherever it shows), the password of `scheme://user:password@host`, and secret-named query
/// parameters (`?api_key=…`, `&token=…`). The rest stays as it was, file names included.
pub fn scrub(text: &str) -> std::borrow::Cow<'_, str> {
    static RES: OnceLock<[regex::Regex; 3]> = OnceLock::new();
    let [loopback, userinfo, query] = RES.get_or_init(|| {
        [
            r#"(https?://(?:127\.0\.0\.1|localhost|\[::1\])(?::\d+)?/[fc]/)[^/\s"'<>]+"#,
            r#"(?i)\b([a-z][a-z0-9+.\-]*://[^/\s:@"'<>]*:)[^/\s@"'<>]+@"#,
            r#"(?i)([?&](?:api[_-]?key|apikey|access[_-]?token|token|password|passwd|pass|pwd|secret|sig|signature|auth|x-emby-token|x-plex-token)=)[^&\s"'<>#]+"#,
        ]
        .map(|p| regex::Regex::new(p).expect("scrub pattern"))
    });
    let known = crate::storage::vfs::loopback_secret().filter(|v| text.contains(*v));
    if known.is_none() && !text.contains("://") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut s = text.to_string();
    if let Some(v) = known {
        s = s.replace(v, "[redacted]");
    }
    s = loopback.replace_all(&s, "${1}[redacted]").into_owned();
    s = userinfo.replace_all(&s, "${1}[redacted]@").into_owned();
    s = query.replace_all(&s, "${1}[redacted]").into_owned();
    std::borrow::Cow::Owned(s)
}

/// Secrets out of a line: the known values, then anything that looks like one.
pub fn redact(line: &str, secrets: &[String]) -> String {
    static RES: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let mut s = line.to_string();
    for v in secrets.iter().filter(|v| v.len() >= 4) {
        if s.contains(v.as_str()) {
            s = s.replace(v.as_str(), "[redacted]");
        }
    }
    let res = RES.get_or_init(|| {
        [
            // Moonrise tokens, bearer headers.
            r"moonrise_[A-Za-z0-9_\-]{16,}",
            r"(?i)(bearer\s+)[A-Za-z0-9._\-]{8,}",
            // key=value / "key": "value" for anything secret-named.
            r#"(?i)((?:api[_-]?key|apikey|token|password|passwd|secret|authorization|x-moonrise-admin)["']?\s*[:=]\s*["']?)[^\s"',;&}]+"#,
            // Long hex or base64-ish runs (hashes, keys).
            r"\b[A-Fa-f0-9]{32,}\b",
            r"\b[A-Za-z0-9+_\-]{40,}={0,2}",
        ]
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
    });
    for (i, re) in res.iter().enumerate() {
        s = match i {
            1 | 2 => re.replace_all(&s, "${1}[redacted]").into_owned(),
            _ => re.replace_all(&s, "[redacted]").into_owned(),
        };
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Taken by the tests here that write to the process-wide buffer: `tail` answers with its
    /// last lines, so a line from a test running alongside would push out the ones a test wrote.
    fn only_writer() -> std::sync::MutexGuard<'static, ()> {
        static WRITER: Mutex<()> = Mutex::new(());
        WRITER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn redaction() {
        let secrets = vec!["s3cret-pass".to_string(), "abcd1234efgh".to_string()];
        let l = "2026-11-05T10:00:00Z  WARN moonrise_server::x: login with password=hunter22 key abcd1234efgh pw s3cret-pass";
        let r = redact(l, &secrets);
        assert!(
            !r.contains("hunter22") && !r.contains("abcd1234efgh") && !r.contains("s3cret-pass"),
            "{r}"
        );
        assert!(r.contains("WARN moonrise_server::x: login with password=[redacted]"), "{r}");
        let r = redact("Authorization: Bearer moonrise_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefg ok", &[]);
        assert!(!r.contains("moonrise_AbCd") && r.ends_with(" ok"), "{r}");
        let r = redact(r#"{"api_key": "zzzzzzzzzz", "x": 1} hash 0123456789abcdef0123456789abcdef01"#, &[]);
        assert!(!r.contains("zzzzzzzzzz") && !r.contains("0123456789abcdef0123"), "{r}");
        // Ordinary lines stay as they are.
        let l = "2026-11-05T10:00:00.123Z  INFO moonrise_server::library: library pass full=true identified=3 ms=5012";
        assert_eq!(redact(l, &secrets), l);
    }

    #[test]
    fn scrub_urls() {
        // A clearly fake value in the place of the loopback secret.
        let fake = "fake-loopback-value";
        // ffmpeg failing to open a share's file through the loopback.
        let l = format!("[in#0 @ 0x55d0] Error opening input: http://127.0.0.1:41234/c/{fake}/7/c21iOi8vbmFzL00vYS5ta3Y/a.mkv: Server returned 404 Not Found");
        let r = scrub(&l);
        assert!(!r.contains(fake), "{r}");
        assert!(
            r.contains("http://127.0.0.1:41234/c/[redacted]/7/c21iOi8vbmFzL00vYS5ta3Y/a.mkv: Server returned 404"),
            "{r}"
        );
        let r = scrub(&format!(
            "ffmpeg -recv_buffer_size 131072 -i http://127.0.0.1:41234/f/{fake}/c21i/a%20b.mkv -c copy"
        ))
        .into_owned();
        assert_eq!(
            r,
            "ffmpeg -recv_buffer_size 131072 -i http://127.0.0.1:41234/f/[redacted]/c21i/a%20b.mkv -c copy"
        );
        // Passwords in URLs, and secret-named parameters.
        let r = scrub("open smb://media:Hunter2!@nas/Media/a.mkv and http://u:p%40ss@host:8096/x failed");
        assert_eq!(
            r,
            "open smb://media:[redacted]@nas/Media/a.mkv and http://u:[redacted]@host:8096/x failed"
        );
        let r = scrub("GET https://api.example/v1/x?api_key=k123456&lang=en&X-Emby-Token=tok999 404");
        assert_eq!(
            r,
            "GET https://api.example/v1/x?api_key=[redacted]&lang=en&X-Emby-Token=[redacted] 404"
        );
        // A user without a password, an ordinary URL, a local path, a line without a URL: as they are.
        for l in [
            "http://user@host/x/f/abc",
            "smb://nas/Media/movies/f/x.mkv",
            "/media/movies/c/abc/a.mkv: No such file or directory",
            "2026-11-05T10:00:00Z  INFO moonrise_server: transcode run item=abc from=3",
            "https://example.org/path?lang=en&page=2",
        ] {
            assert_eq!(scrub(l), l);
        }
    }

    #[tokio::test]
    async fn scrub_known_loopback_secret() {
        crate::storage::vfs::start_loopback().await.unwrap();
        let secret = crate::storage::vfs::loopback_secret().unwrap();
        // The real loopback URL ffmpeg gets, and the value on its own.
        let url = crate::storage::vfs::ffmpeg_input("smb://nas/Media/movies/a.mkv");
        assert!(url.contains(secret));
        let r = scrub(&format!("{url}: Input/output error; secret {secret} seen")).into_owned();
        assert!(
            !r.contains(secret) && r.contains("/f/[redacted]/") && r.ends_with("a.mkv: Input/output error; secret [redacted] seen"),
            "{r}"
        );
    }

    #[test]
    fn tee_scrubs_before_writing() {
        let _writer = only_writer();
        let mut t = Tee::default();
        t.write_all(b"2026-11-05T10:00:00Z  WARN moonrise_server: ffmpeg: http://127.0.0.1:1/c/s3cretvalue/1/aWQ/a.mkv: ")
            .unwrap();
        t.write_all(b"404 tee-scrub-marker\n").unwrap();
        drop(t);
        let all = tail(MAX_LINES, None, &[]);
        let l = all.iter().find(|l| l.contains("tee-scrub-marker")).expect("line kept");
        assert!(!l.contains("s3cretvalue") && l.contains("/c/[redacted]/1/aWQ/a.mkv"), "{l}");
    }

    #[test]
    fn tail_and_levels() {
        let _writer = only_writer();
        let mut t = Tee::default();
        t.write_all(b"2026-11-05T10:00:00Z  INFO moonrise_server: one\n2026-11-05T10:00:01Z  WARN moonrise_server: two\npartial")
            .unwrap();
        drop(t);
        let one = "2026-11-05T10:00:00Z  INFO moonrise_server: one";
        let two = "2026-11-05T10:00:01Z  WARN moonrise_server: two";
        assert_eq!(tail(3, None, &[]), [one, two, "partial"]);
        assert_eq!(tail(2, None, &[]), [two, "partial"]);
        let w = tail(10, Some("warn"), &[]);
        assert!(w.iter().all(|l| level_of(l) == Some("WARN") || level_of(l) == Some("ERROR")));
        assert_eq!(w.last().map(String::as_str), Some(two));
    }
}
