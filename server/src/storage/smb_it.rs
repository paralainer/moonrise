//! Integration test against a real Samba server (ignored by default): `tools/smb-test.sh` starts
//! one in Docker with a fixture folder and runs `cargo test -- --ignored smb_` with
//!
//! - `MOONRISE_SMB_TEST_HOST` (`127.0.0.1:1445`), `MOONRISE_SMB_TEST_USER`, `MOONRISE_SMB_TEST_PASS`
//! - `MOONRISE_SMB_TEST_FIXTURE`: the folder the share `media` serves (to compare bytes and sizes);
//!   the share `public` is open to guests.

use std::{collections::HashMap, collections::HashSet, time::Duration};

use futures_util::StreamExt;

use crate::{
    admin::config::LibraryConfig,
    storage::smb::{self, Creds, Problem, ShareConfig, SmbPath},
    storage::vfs,
};

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} (run tools/smb-test.sh)"))
}

fn fixture_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            fixture_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Samba server (server/tools/smb-test.sh)"]
async fn smb_against_samba() {
    let host = env("MOONRISE_SMB_TEST_HOST");
    let user = env("MOONRISE_SMB_TEST_USER");
    let pass = env("MOONRISE_SMB_TEST_PASS");
    let fixture = std::path::PathBuf::from(env("MOONRISE_SMB_TEST_FIXTURE"));
    let good = Creds {
        username: user.clone(),
        password: pass.clone(),
        domain: String::new(),
    };
    let wrong = Creds {
        password: "not-the-password".into(),
        ..good.clone()
    };

    // Shares: listed with the account; guest sees them too (map to guest), a wrong password
    // and an unreachable server are told apart.
    let shares = smb::list_shares(&host, &good, false).await.unwrap();
    let names: Vec<&str> = shares.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"media") && names.contains(&"public"), "{names:?}");
    assert!(!names.iter().any(|n| n.ends_with('$')), "no hidden shares");
    let e = smb::list_shares(&host, &wrong, false).await.unwrap_err();
    assert_eq!(smb::problem_of(&e), Problem::WrongPassword, "{e:#}");
    let unknown = Creds {
        username: "nobody-here".into(),
        password: "x".into(),
        domain: String::new(),
    };
    let e = smb::list_shares(&host, &unknown, false).await.unwrap_err();
    assert_eq!(
        smb::problem_of(&e),
        Problem::WrongPassword,
        "an unknown user mapped to guest is refused: {e:#}"
    );
    let e = smb::list_shares("127.0.0.1:1", &good, false).await.unwrap_err();
    assert_eq!(smb::problem_of(&e), Problem::Unreachable, "{e:#}");
    // The private share needs the account; the public one opens as guest.
    let e = smb::Share::connect(&host, "media", &Creds::guest(), false)
        .await
        .err()
        .expect("guest refused");
    assert_eq!(smb::problem_of(&e), Problem::AuthRequired, "{e:#}");
    // No such share: Samba says so to a signed session, and "access denied" to an unsigned one.
    let e = smb::Share::connect(&host, "nope", &good, true).await.err().expect("no such share");
    assert_eq!(smb::problem_of(&e), Problem::NotFound, "{e:#}");
    let e = smb::Share::connect(&host, "nope", &good, false).await.err().expect("no such share");
    assert!(matches!(smb::problem_of(&e), Problem::NotFound | Problem::AccessDenied), "{e:#}");
    let public = smb::Share::connect(&host, "public", &Creds::guest(), false).await.unwrap();
    assert!(!public.list("").await.unwrap().is_empty());
    // Signed sessions work too.
    let signed = smb::Share::connect(&host, "media", &good, true).await.unwrap();
    assert!(!signed.list("").await.unwrap().is_empty());

    // The server's client, configured as the settings would.
    let client = smb::client();
    client.configure(
        &[ShareConfig {
            host: host.clone(),
            share: "media".into(),
            creds: Some(good.clone()),
        }],
        false,
    );
    vfs::start_loopback().await.unwrap();
    let root = SmbPath::new(&host, "media", "").unwrap().url();
    let mut files = Vec::new();
    fixture_files(&fixture, &mut files);
    assert!(files.len() >= 3);
    for f in &files {
        let rel = f.strip_prefix(&fixture).unwrap().to_string_lossy().to_string();
        let url = format!("{root}/{rel}");
        let local = std::fs::read(f).unwrap();
        let meta = std::fs::metadata(f).unwrap();
        let m = vfs::stat(&url).await.unwrap();
        assert!(m.is_file && !m.is_dir);
        assert_eq!(m.size, local.len() as u64, "{rel}");
        let local_ns = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64;
        assert_eq!(
            m.mtime / 100,
            local_ns / 100,
            "{rel}: mtime as the kernel's client reports it (100 ns ticks)"
        );
        // Reads at offsets, across the 1 MiB pieces and past the end.
        let n = local.len() as u64;
        for (off, len) in [(0, 10), (n / 3, 3 << 20), (n.saturating_sub(5), 100), ((1 << 20) - 7, 14), (n, 10)] {
            let got = vfs::read_at(&url, off, len).await.unwrap();
            let s = off.min(n) as usize;
            let e = (off + len).min(n) as usize;
            assert_eq!(&got[..], &local[s..e], "{rel} @{off}+{len}");
        }
        // Streamed whole, and through the loopback listener ffmpeg reads (a range).
        let s: Vec<bytes::Bytes> = vfs::stream(&url, 0, n).await.unwrap().map(|b| b.unwrap()).collect().await;
        assert_eq!(s.concat(), local, "{rel} streamed");
        let input = vfs::ffmpeg_input(&url);
        assert!(input.starts_with("http://127.0.0.1:"), "{input}");
        let from = n / 2;
        let r = reqwest::Client::new()
            .get(&input)
            .header("range", format!("bytes={from}-"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(&r.bytes().await.unwrap()[..], &local[from as usize..], "{rel} via loopback");
    }
    let bad_secret = vfs::ffmpeg_input(&format!("{root}/x")).replacen("/f/", "/f/0", 1);
    assert_eq!(reqwest::get(bad_secret).await.unwrap().status(), 404);

    // Listings, and a scan of the share as a library root.
    let movies = format!("{root}/movies");
    let listed = vfs::read_dir(&movies).await.unwrap();
    assert!(listed.iter().any(|e| e.is_dir), "{listed:?}");
    let libs = vec![LibraryConfig {
        id: "m".into(),
        name: "M".into(),
        kind: "movies".into(),
        roots: vec![movies.clone(), format!("{root}/missing-folder")],
    }];
    let fs = tokio::task::spawn_blocking(move || crate::library::scanner::scan_fs(&libs, true, &HashMap::new(), &[], &HashSet::new()))
        .await
        .unwrap();
    assert_eq!(fs.online, vec![movies.clone()]);
    assert_eq!(fs.offline.len(), 1, "a missing folder is offline, not fatal");
    let local_videos = files
        .iter()
        .filter(|f| f.starts_with(fixture.join("movies")) && crate::library::parse::is_video(&f.to_string_lossy()))
        .count();
    assert_eq!(fs.files.len(), local_videos);
    for (_, path, size, mtime) in &fs.files {
        let rel = path.strip_prefix(&format!("{root}/")).unwrap();
        let meta = std::fs::metadata(fixture.join(rel)).unwrap();
        assert_eq!(*size, meta.len());
        assert!(*mtime > 0);
    }
    assert!(fs.dir_mtimes.iter().any(|(d, _)| d == &movies));

    // Wrong credentials: the root is offline (no files lost), with the reason.
    client.configure(
        &[ShareConfig {
            host: host.clone(),
            share: "media".into(),
            creds: Some(wrong.clone()),
        }],
        false,
    );
    let libs = vec![LibraryConfig {
        id: "m".into(),
        name: "M".into(),
        kind: "movies".into(),
        roots: vec![movies.clone()],
    }];
    let fs = tokio::task::spawn_blocking(move || crate::library::scanner::scan_fs(&libs, true, &HashMap::new(), &[], &HashSet::new()))
        .await
        .unwrap();
    assert!(fs.online.is_empty() && fs.files.is_empty());
    assert!(fs.offline[0].1.contains("password"), "{:?}", fs.offline);
    // Back to the right ones: works again after the retry pause.
    client.configure(
        &[ShareConfig {
            host: host.clone(),
            share: "media".into(),
            creds: Some(good),
        }],
        false,
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(vfs::read_dir(&movies).await.is_ok());
}

/// A TCP proxy to `to` holding each piece of the answers back for a moment. Its `host:port`.
async fn slow_proxy(to: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let to = to.to_string();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((client, _)) = listener.accept().await {
            let server = tokio::net::TcpStream::connect(&to).await.unwrap();
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            tokio::spawn(async move { tokio::io::copy(&mut cr, &mut sw).await });
            tokio::spawn(async move {
                let mut buf = vec![0u8; 256 << 10];
                while let Ok(n @ 1..) = sr.read(&mut buf).await {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

/// A reader that stopped reading holds nothing back. A stream reads ahead, and ffmpeg's reads
/// through the loopback listener can stop for as long as it likes (it is paused, or whatever it
/// writes to is full); others want the same blocks of the cache meanwhile. Its own process, for a
/// cold cache: `smb_against_samba` has read everything.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Samba server (server/tools/smb-test.sh)"]
async fn smb_stopped_reader_holds_nothing() {
    let host = env("MOONRISE_SMB_TEST_HOST");
    let fixture = std::path::PathBuf::from(env("MOONRISE_SMB_TEST_FIXTURE"));
    let good = Creds {
        username: env("MOONRISE_SMB_TEST_USER"),
        password: env("MOONRISE_SMB_TEST_PASS"),
        domain: String::new(),
    };
    // Answers as slow as a NAS's: from a Samba on this machine they would all be in before the
    // reader stops, and nothing would be left in flight.
    let host = slow_proxy(&host).await;
    smb::client().configure(
        &[ShareConfig {
            host: host.clone(),
            share: "media".into(),
            creds: Some(good),
        }],
        false,
    );
    let local = std::fs::read(fixture.join("movies/blob.bin")).unwrap();
    let url = format!("{}/movies/blob.bin", SmbPath::new(&host, "media", "").unwrap().url());
    let mut stopped = vfs::stream(&url, 0, local.len() as u64).await.unwrap();
    // Far enough for the read-ahead to have pieces in flight, then no more polls.
    let mut taken = Vec::new();
    while taken.len() < 2 << 20 {
        taken.extend_from_slice(&stopped.next().await.unwrap().unwrap());
    }
    // What it has in flight is what comes next.
    let at = taken.len();
    let read = vfs::read_at(&url, at as u64, 3 << 20);
    let got = tokio::time::timeout(Duration::from_secs(20), read)
        .await
        .expect("a stopped reader holds back another's read")
        .unwrap();
    assert_eq!(&got[..], &local[at..at + (3 << 20)]);
    // And it can go on when it wakes up.
    while taken.len() < local.len() {
        taken.extend_from_slice(&stopped.next().await.unwrap().unwrap());
    }
    assert_eq!(taken, local);
}

/// The share goes away under a long-lived client (the script restarts Samba between the two
/// halves, `MOONRISE_SMB_TEST_CTL` is the folder the two sides signal through): reads fail while
/// it is down and work again without a restart of ours.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Samba server (server/tools/smb-test.sh)"]
async fn smb_reconnects() {
    let host = env("MOONRISE_SMB_TEST_HOST");
    let ctl = std::path::PathBuf::from(env("MOONRISE_SMB_TEST_CTL"));
    let fixture = std::path::PathBuf::from(env("MOONRISE_SMB_TEST_FIXTURE"));
    let good = Creds {
        username: env("MOONRISE_SMB_TEST_USER"),
        password: env("MOONRISE_SMB_TEST_PASS"),
        domain: String::new(),
    };
    let client = smb::client();
    client.configure(
        &[ShareConfig {
            host: host.clone(),
            share: "media".into(),
            creds: Some(good),
        }],
        false,
    );
    let mut files = Vec::new();
    fixture_files(&fixture, &mut files);
    // The big file: a small read in its middle isn't cached (it goes to the share each time).
    let f = files.iter().find(|f| f.extension().is_some_and(|e| e == "bin")).unwrap();
    let rel = f.strip_prefix(&fixture).unwrap().to_string_lossy().to_string();
    let url = format!("{}/{rel}", SmbPath::new(&host, "media", "").unwrap().url());
    let local = std::fs::read(f).unwrap();
    let mid = 5_000_000usize;
    assert_eq!(&vfs::read_at(&url, mid as u64, 5000).await.unwrap()[..], &local[mid..mid + 5000]);
    std::fs::write(ctl.join("ready"), b"1").unwrap();
    for _ in 0..600 {
        if ctl.join("restarted").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ctl.join("restarted").exists(), "the script didn't restart Samba");
    // Back within a few tries (the client re-dials; a dead handle is opened again; a share that
    // failed to connect is tried again after 5 s).
    let t0 = std::time::Instant::now();
    let movies = SmbPath::new(&host, "media", "movies").unwrap().url();
    let mut last = None;
    for i in 0..40 {
        let at = mid + 100_000 * (i + 1);
        match (vfs::read_at(&url, at as u64, 5000).await, vfs::read_dir(&movies).await) {
            (Ok(b), Ok(l)) => {
                assert_eq!(&b[..], &local[at..at + 5000]);
                assert!(!l.is_empty());
                last = None;
                break;
            }
            (a, b) => last = Some(format!("{:?} / {:?}", a.err(), b.err())),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(last.is_none(), "still failing after the restart: {last:?}");
    eprintln!("reads back {:.1} s after the restart", t0.elapsed().as_secs_f64());
}
