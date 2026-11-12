//! Golden test for the command line (`cli::run`): every subcommand, with good and bad
//! arguments, run as the binary in a scratch data folder. The subcommands that talk to a running
//! server talk to a stand-in that records what it was asked and answers `{"ok": true}`.

use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

#[path = "../src/golden.rs"]
mod golden;

/// A stand-in server: answers every request with `{"ok": true}`, keeping the requests.
fn stand_in() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // The head, then as much body as it announces.
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if buf.len() >= head_end + 4 + len {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&buf).to_string();
            let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
            let mut lines = head.lines();
            let request = lines.next().unwrap_or("").to_string();
            let mut headers: Vec<String> = lines
                .filter(|l| {
                    let k = l.split(':').next().unwrap_or("").to_ascii_lowercase();
                    k == "x-moonrise-admin" || k == "content-type"
                })
                .map(str::to_ascii_lowercase)
                .collect();
            headers.sort();
            s.lock().unwrap().push(format!("{request} {headers:?} {body}"));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}",
            );
        }
    });
    (port, seen)
}

fn run(dir: &Path, port: u16, args: &[&str], stdin: &str) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_moonrise-server"));
    cmd.args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("RUST_LOG", "off")
        .env("MOONRISE_DATA_DIR", dir.join("data"))
        .env("MOONRISE_CACHE_DIR", dir.join("cache"))
        .env("MOONRISE_LISTEN", format!("127.0.0.1:{port}"))
        .env("MOONRISE_FFMPEG", "/nonexistent/ffmpeg")
        .env("MOONRISE_FFPROBE", "/nonexistent/ffprobe")
        .env("TMPDIR", dir.join("tmp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    format!(
        "$ {}\nexit {:?}\n--- stdout\n{}--- stderr\n{}",
        args.join(" "),
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Times, versions and ports made stable.
fn normalise(s: &str, dir: &Path, port: u16) -> String {
    let s = s
        .replace(&dir.to_string_lossy().into_owned(), "<dir>")
        .replace(env!("CARGO_PKG_VERSION"), "<version>")
        .replace(&format!(":{port}"), ":<port>");
    let times = regex::Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)").unwrap();
    let s = times.replace_all(&s, "<time>").into_owned();
    // Random ids (UUIDs, ULIDs, tokens).
    let ids = regex::Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b|\b[0-9a-z]{26}\b").unwrap();
    ids.replace_all(&s, "<id>").into_owned()
}

#[test]
fn cli_golden() {
    let dir = std::env::temp_dir().join(format!("moonrise-cli-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tmp")).unwrap();
    let (port, seen) = stand_in();
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["--version"], ""),
        (vec!["-V"], ""),
        (vec!["version"], ""),
        (vec!["no-such-command"], ""),
        // No admin token yet: nothing is asked of the server.
        (vec!["scan"], ""),
        (vec!["user", "list"], ""),
        (vec!["user", "add"], ""),
        (vec!["user", "add", "alice", "--admin"], "a-long-password-1\n"),
        (vec!["user", "add", "bob"], "another-password-2\n"),
        (vec!["user", "passwd", "bob"], "a-third-password-3\n"),
        (vec!["user", "list"], ""),
        (vec!["user", "revoke", "bob"], ""),
        (vec!["user", "delete", "bob"], ""),
        (vec!["user", "delete", "bob", "--yes"], ""),
        (vec!["user", "frobnicate"], ""),
        (vec!["users", "reset-data"], ""),
        (vec!["users", "reset-data", "--user", "alice", "--yes"], ""),
        (vec!["users", "list"], ""),
        (vec!["llm-test"], ""),
        (vec!["llm-replay", "3"], ""),
        (vec!["align-eval"], ""),
        (vec!["align-parts", "--apply"], ""),
        (vec!["identify-eval"], ""),
        (vec!["identify-eval", "--typed"], ""),
        (vec!["migrate-root"], ""),
        (vec!["migrate-root", "/media/a"], ""),
        (vec!["migrate-root", "/media/a", "smb://nas/share", "--credential", "c1"], ""),
        (vec!["merge-libraries"], ""),
        (vec!["merge-libraries", "films", "shows", "--name", "Everything"], ""),
        (vec!["subs-test"], ""),
        (vec!["keyframes-test", "/nonexistent/a.mkv"], ""),
        (vec!["credits-test"], ""),
        (vec!["smb-credential"], ""),
        (vec!["smb-credential", "add"], ""),
    ];
    let mut out = String::new();
    for (args, stdin) in &cases {
        out.push_str(&run(&dir, port, args, stdin));
    }
    // With an admin token: the subcommands that ask the running server.
    std::fs::create_dir_all(dir.join("data/secrets")).unwrap();
    std::fs::write(dir.join("data/secrets/admin.token"), "test-admin-token\n").unwrap();
    let asking: Vec<(Vec<&str>, &str)> = vec![
        (vec!["scan"], ""),
        (vec!["scan", "--full"], ""),
        (vec!["stats"], ""),
        (vec!["stats", "--recent", "12"], ""),
        (
            vec!["smb-credential", "add", "--user", "media", "--host", "nas", "--domain", "HOME"],
            "share-password\n",
        ),
        (vec!["smb-credential", "add", "--user", "media"], "share-password\n"),
        (vec!["healthcheck"], ""),
    ];
    for (args, stdin) in &asking {
        out.push_str(&run(&dir, port, args, stdin));
    }
    out.push_str("--- the stand-in server was asked\n");
    for r in seen.lock().unwrap().iter() {
        out.push_str(r);
        out.push('\n');
    }
    let out = normalise(&out, &dir, port);
    let _ = std::fs::remove_dir_all(&dir);
    golden::check("cli/commands", &out);
}
