//! File access for media paths, local or on an SMB share (`smb://host/share/…`, see `smb.rs`):
//! metadata, listings, reads at an offset, streams, and the input ffmpeg/ffprobe open.
//!
//! Local paths go straight to the file system as before. ffmpeg can't read SMB (the
//! jellyfin-ffmpeg build has no libsmbclient), so for a share it is handed a URL on a loopback
//! HTTP listener of this process (`127.0.0.1`, a random port and a random secret in the path),
//! which answers `Range` requests from the SMB client: ffmpeg seeks by asking for another range.
//!
//! The `_blocking` variants are for code that runs on blocking threads (the scanner): they wait
//! on the runtime for SMB paths.

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use bytes::Bytes;
use futures_util::{stream::BoxStream, StreamExt};

use crate::storage::smb::{self, SmbPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    pub is_dir: bool,
    pub is_file: bool,
    pub size: u64,
    /// ns since the epoch.
    pub mtime: i64,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// The entry's full path (same kind as the folder's).
    pub path: String,
    pub is_dir: bool,
    /// A plain file (not a link, device…).
    pub is_file: bool,
    pub size: u64,
    pub mtime: i64,
}

pub fn is_smb(path: &str) -> bool {
    smb::is_smb(path)
}

fn smb_path(path: &str) -> std::io::Result<SmbPath> {
    SmbPath::parse(path).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{path}: not an smb:// path")))
}

fn mtime_ns(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64)
}

fn local_meta(m: &std::fs::Metadata) -> Meta {
    Meta {
        is_dir: m.is_dir(),
        is_file: m.is_file(),
        size: m.len(),
        mtime: mtime_ns(m),
    }
}

fn smb_meta(m: smb::Meta) -> Meta {
    Meta {
        is_dir: m.is_dir,
        is_file: !m.is_dir,
        size: if m.is_dir { 0 } else { m.size },
        mtime: m.mtime,
    }
}

/// The folder a path is in.
pub fn parent(path: &str) -> Option<String> {
    let p = Path::new(path).parent()?.to_string_lossy().into_owned();
    if is_smb(path) && SmbPath::parse(&p).is_none() {
        return None;
    }
    Some(p)
}

pub fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A name in a folder.
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

// ---------------------------------------------------------------- metadata, listings

pub async fn stat(path: &str) -> std::io::Result<Meta> {
    if is_smb(path) {
        let p = smb_path(path)?;
        return smb::client().stat(&p).await.map(smb_meta).map_err(smb::io_error);
    }
    tokio::fs::metadata(path).await.map(|m| local_meta(&m))
}

/// `stat` for a file about to be read: on a share, from its open handle (no round trip; as it
/// was when opened, at most a minute ago).
pub async fn stat_for_read(path: &str) -> std::io::Result<Meta> {
    if is_smb(path) {
        let p = smb_path(path)?;
        return smb::client().file_meta(&p).await.map(smb_meta).map_err(smb::io_error);
    }
    stat(path).await
}

pub fn stat_blocking(path: &str) -> std::io::Result<Meta> {
    if is_smb(path) {
        return block_on(stat(path));
    }
    std::fs::metadata(path).map(|m| local_meta(&m))
}

/// Whether a path is a file that can be read now.
pub async fn is_file(path: &str) -> bool {
    stat(path).await.is_ok_and(|m| m.is_file)
}

pub async fn read_dir(path: &str) -> std::io::Result<Vec<Entry>> {
    if is_smb(path) {
        let p = smb_path(path)?;
        let list = smb::client().list(&p).await.map_err(smb::io_error)?;
        return Ok(list
            .into_iter()
            .map(|e| Entry {
                path: p.join(&e.name).url(),
                is_dir: e.is_dir && !e.is_link,
                is_file: !e.is_dir && !e.is_link,
                size: if e.is_dir { 0 } else { e.size },
                mtime: e.mtime,
                name: e.name,
            })
            .collect());
    }
    let path = path.to_string();
    tokio::task::spawn_blocking(move || read_dir_local(&path))
        .await
        .map_err(std::io::Error::other)?
}

pub fn read_dir_blocking(path: &str) -> std::io::Result<Vec<Entry>> {
    if is_smb(path) {
        return block_on(read_dir(path));
    }
    read_dir_local(path)
}

/// Entries as `read_dir` sees them: links are neither folders nor files (not followed).
fn read_dir_local(path: &str) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(path)?.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let (size, mtime) = if ft.is_file() {
            e.metadata().map_or((0, 0), |m| (m.len(), mtime_ns(&m)))
        } else {
            (0, 0)
        };
        out.push(Entry {
            path: e.path().to_string_lossy().into_owned(),
            name,
            is_dir: ft.is_dir(),
            is_file: ft.is_file(),
            size,
            mtime,
        });
    }
    Ok(out)
}

/// A small file whole (sidecar subtitles).
pub async fn read(path: &str) -> std::io::Result<Vec<u8>> {
    if is_smb(path) {
        let p = smb_path(path)?;
        let smb = smb::client();
        let m = smb.file_meta(&p).await.map_err(smb::io_error)?;
        if m.size > 64 << 20 {
            return Err(std::io::Error::other(format!("{path}: too big to read whole")));
        }
        return smb.read_at(&p, 0, m.size).await.map(|b| b.to_vec()).map_err(smb::io_error);
    }
    tokio::fs::read(path).await
}

// ---------------------------------------------------------------- reads

/// `len` bytes at `offset` (fewer at the end of the file).
pub async fn read_at(path: &str, offset: u64, len: u64) -> Result<Bytes> {
    if is_smb(path) {
        let p = SmbPath::parse(path).ok_or_else(|| anyhow::anyhow!("{path}: not an smb:// path"))?;
        return smb::client().read_at(&p, offset, len).await;
    }
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || -> Result<Bytes> {
        use std::os::unix::fs::FileExt;
        let f = std::fs::File::open(&path)?;
        let mut buf = vec![0u8; len as usize];
        let mut got = 0usize;
        while got < buf.len() {
            let n = f.read_at(&mut buf[got..], offset + got as u64)?;
            if n == 0 {
                break;
            }
            got += n;
        }
        buf.truncate(got);
        Ok(Bytes::from(buf))
    })
    .await?
}

/// The bytes [start, start+len) as a stream.
pub async fn stream(path: &str, start: u64, len: u64) -> std::io::Result<BoxStream<'static, std::io::Result<Bytes>>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    if is_smb(path) {
        let p = smb_path(path)?;
        return Ok(smb::client().stream(p, start, len).boxed());
    }
    let mut f = tokio::fs::File::open(path).await?;
    if start > 0 {
        f.seek(std::io::SeekFrom::Start(start)).await?;
    }
    Ok(tokio_util::io::ReaderStream::with_capacity(f.take(len), 256 << 10).boxed())
}

/// Runs an SMB call from a blocking thread.
fn block_on<T>(f: impl std::future::Future<Output = std::io::Result<T>>) -> std::io::Result<T> {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => h.block_on(f),
        Err(_) => Err(std::io::Error::other("no runtime for an SMB call")),
    }
}

// ---------------------------------------------------------------- ffmpeg's input

struct Loopback {
    addr: SocketAddr,
    secret: String,
}

static LOOPBACK: OnceLock<Loopback> = OnceLock::new();

/// The input arguments of ffmpeg/ffprobe for a path (`-i path`). For a share, the loopback URL,
/// with a small receive buffer: the listener reads ahead from the share as ffmpeg takes the
/// bytes, and kernel buffers would otherwise take megabytes it never reads (every ffmpeg run
/// reads a few hundred KB of headers from three or four places before its data).
pub fn ffmpeg_input_args(path: &str) -> Vec<String> {
    if is_smb(path) {
        return vec!["-recv_buffer_size".into(), "131072".into(), "-i".into(), ffmpeg_input(path)];
    }
    vec!["-i".into(), path.to_string()]
}

/// The secret in the loopback URLs (None before the listener starts), for `logbuf::scrub`.
pub fn loopback_secret() -> Option<&'static str> {
    LOOPBACK.get().map(|lb| lb.secret.as_str())
}

/// What ffmpeg/ffprobe open for a path: the path, or the loopback URL of a file on a share.
pub fn ffmpeg_input(path: &str) -> String {
    loopback_url(path, None).unwrap_or_else(|| path.to_string())
}

/// `ffmpeg_input_args`, and for a share a count of the bytes the loopback has sent this input
/// (None for a local path: the reader's own `rchar` counts those, see `segment.rs`).
pub fn ffmpeg_input_args_counted(path: &str) -> (Vec<String>, Option<ReadCount>) {
    if !is_smb(path) {
        return (ffmpeg_input_args(path), None);
    }
    let count = ReadCount::new();
    match loopback_url(path, Some(count.id)) {
        Some(url) => (vec!["-recv_buffer_size".into(), "131072".into(), "-i".into(), url], Some(count)),
        None => (ffmpeg_input_args(path), None),
    }
}

/// The loopback URL of a file on a share (None: not one, or no listener). With `count`, the
/// bytes sent for it are added to that `ReadCount`.
fn loopback_url(path: &str, count: Option<u64>) -> Option<String> {
    use base64::Engine;
    if !is_smb(path) {
        return None;
    }
    let Some(lb) = LOOPBACK.get() else {
        tracing::warn!("SMB input asked for before the loopback listener started");
        return None;
    };
    let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.as_bytes());
    // The file name last, so ffmpeg can go by its extension where the content doesn't say.
    let name = percent(&file_name(path));
    Some(match count {
        None => format!("http://{}/f/{}/{id}/{name}", lb.addr, lb.secret),
        Some(c) => format!("http://{}/c/{}/{c}/{id}/{name}", lb.addr, lb.secret),
    })
}

/// How many bytes the loopback listener has sent one reader (an ffmpeg run) of a file on a
/// share, over all its requests: how a run that writes nothing for a long time (an audio run
/// writes all it has at its end) shows that it is still reading. Counting stops when dropped.
pub struct ReadCount {
    id: u64,
    bytes: Arc<AtomicU64>,
}

static READ_COUNTS: Mutex<Option<HashMap<u64, Arc<AtomicU64>>>> = Mutex::new(None);

impl ReadCount {
    pub fn new() -> ReadCount {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let bytes = Arc::new(AtomicU64::new(0));
        READ_COUNTS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(id, bytes.clone());
        ReadCount { id, bytes }
    }

    pub fn get(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// The counter itself, to add to by hand.
    #[cfg(test)]
    pub fn counter(&self) -> Arc<AtomicU64> {
        self.bytes.clone()
    }

    fn of(id: u64) -> Option<Arc<AtomicU64>> {
        READ_COUNTS.lock().unwrap().as_ref()?.get(&id).cloned()
    }
}

impl Default for ReadCount {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ReadCount {
    fn drop(&mut self) {
        if let Some(m) = READ_COUNTS.lock().unwrap().as_mut() {
            m.remove(&self.id);
        }
    }
}

/// A reader's pace through the loopback listener (a file on a share): at most `rate` bytes a
/// second, 0 for as fast as it reads. Background reads of whole files (`scan.rs`) go at a pace,
/// so they never take much of the share. Pacing stops when dropped.
pub struct Pace {
    id: u64,
    inner: Arc<PaceInner>,
}

pub struct PaceInner {
    rate: AtomicU64,
    /// When the next piece may go out.
    next: Mutex<Instant>,
    sent: AtomicU64,
}

impl PaceInner {
    /// Waits for `n` bytes' turn.
    async fn take(&self, n: u64) {
        self.sent.fetch_add(n, Ordering::Relaxed);
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let at = {
            let mut next = self.next.lock().unwrap();
            let now = Instant::now();
            if *next < now {
                *next = now;
            }
            let at = *next;
            *next += Duration::from_secs_f64(n as f64 / rate as f64);
            at
        };
        tokio::time::sleep_until(at.into()).await;
    }
}

static PACES: Mutex<Option<HashMap<u64, Arc<PaceInner>>>> = Mutex::new(None);

impl Pace {
    pub fn new(rate: u64) -> Pace {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::new(PaceInner {
            rate: AtomicU64::new(rate),
            next: Mutex::new(Instant::now()),
            sent: AtomicU64::new(0),
        });
        PACES.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, inner.clone());
        Pace { id, inner }
    }

    /// A new rate, from the next piece on (0: as fast as the reader reads).
    pub fn set_rate(&self, rate: u64) {
        self.inner.rate.store(rate, Ordering::Relaxed);
    }

    /// Waits for `n` bytes' turn, for reads made here rather than through the loopback.
    pub async fn take(&self, n: u64) {
        self.inner.take(n).await;
    }

    /// Bytes sent so far.
    pub fn sent(&self) -> u64 {
        self.inner.sent.load(Ordering::Relaxed)
    }

    fn of(id: u64) -> Option<Arc<PaceInner>> {
        PACES.lock().unwrap().as_ref()?.get(&id).cloned()
    }
}

impl Drop for Pace {
    fn drop(&mut self) {
        if let Some(m) = PACES.lock().unwrap().as_mut() {
            m.remove(&self.id);
        }
    }
}

/// `ffmpeg_input_args`, a file on a share read at `pace` (a local path: as it is).
pub fn ffmpeg_input_args_paced(path: &str, pace: &Pace) -> Vec<String> {
    match LOOPBACK.get().filter(|_| is_smb(path)) {
        Some(lb) => {
            use base64::Engine;
            let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.as_bytes());
            let url = format!("http://{}/p/{}/{}/{id}/{}", lb.addr, lb.secret, pace.id, percent(&file_name(path)));
            vec!["-recv_buffer_size".into(), "131072".into(), "-i".into(), url]
        }
        None => ffmpeg_input_args(path),
    }
}

fn percent(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A loopback request: the file of `id` (its URL, base64) if the secret was right, its bytes
/// added to `count` as they go out.
async fn serve_loopback(
    secret_ok: bool,
    id: &str,
    count: Option<Arc<AtomicU64>>,
    headers: &axum::http::HeaderMap,
) -> axum::response::Response {
    serve_loopback_paced(secret_ok, id, count, None, headers).await
}

/// `serve_loopback`, each piece waiting for its turn at `pace`.
async fn serve_loopback_paced(
    secret_ok: bool,
    id: &str,
    count: Option<Arc<AtomicU64>>,
    pace: Option<Arc<PaceInner>>,
    headers: &axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use base64::Engine;
    let path = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(id.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok());
    let p = match path {
        Some(p) if secret_ok && is_smb(&p) => p,
        _ => return (axum::http::StatusCode::NOT_FOUND, "no").into_response(),
    };
    let resp = crate::downloads::serve_file(&p, headers, "application/octet-stream", &file_name(&p), None).await;
    let resp = match pace {
        Some(pace) => {
            let (parts, body) = resp.into_parts();
            let paced = body.into_data_stream().then(move |chunk| {
                let pace = pace.clone();
                async move {
                    if let Ok(b) = &chunk {
                        pace.take(b.len() as u64).await;
                    }
                    chunk
                }
            });
            axum::response::Response::from_parts(parts, axum::body::Body::from_stream(paced))
        }
        None => resp,
    };
    let Some(count) = count else { return resp };
    let (parts, body) = resp.into_parts();
    let counted = body.into_data_stream().inspect(move |chunk| {
        if let Ok(b) = chunk {
            count.fetch_add(b.len() as u64, Ordering::Relaxed);
        }
    });
    axum::response::Response::from_parts(parts, axum::body::Body::from_stream(counted))
}

/// Starts the loopback listener ffmpeg reads shares through (once).
pub async fn start_loopback() -> Result<()> {
    use axum::serve::ListenerExt;
    use axum::{extract::Path as UrlPath, http::HeaderMap, routing::get, Router};
    if LOOPBACK.get().is_some() {
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let secret: String = (0..24).map(|_| format!("{:x}", rand::random::<u8>() & 15)).collect();
    let want = secret.clone();
    let want2 = want.clone();
    let want3 = want.clone();
    let app = Router::new()
        .route(
            "/f/{secret}/{id}/{name}",
            get(
                move |UrlPath((s, id, _name)): UrlPath<(String, String, String)>, headers: HeaderMap| {
                    let want = want.clone();
                    async move { serve_loopback(s == want, &id, None, &headers).await }
                },
            ),
        )
        .route(
            "/c/{secret}/{count}/{id}/{name}",
            get(
                move |UrlPath((s, count, id, _name)): UrlPath<(String, u64, String, String)>, headers: HeaderMap| {
                    let want = want2.clone();
                    async move { serve_loopback(s == want, &id, ReadCount::of(count), &headers).await }
                },
            ),
        )
        .route(
            "/p/{secret}/{pace}/{id}/{name}",
            get(
                move |UrlPath((s, pace, id, _name)): UrlPath<(String, u64, String, String)>, headers: HeaderMap| {
                    let want = want3.clone();
                    async move {
                        // A pace that is gone (its reader stopped): nothing to serve.
                        match Pace::of(pace) {
                            Some(p) => serve_loopback_paced(s == want, &id, None, Some(p), &headers).await,
                            None => serve_loopback(false, &id, None, &headers).await,
                        }
                    }
                },
            ),
        );
    let _ = LOOPBACK.set(Loopback { addr, secret });
    // Small send buffers too (see `ffmpeg_input_args`).
    let listener = listener.tap_io(|tcp| {
        use std::os::fd::AsRawFd;
        let size: libc::c_int = 128 << 10;
        // SAFETY: the descriptor is the accepted socket's, open for the call (`tcp` is borrowed), and
        // the option value points at a live c_int with its size as the length.
        unsafe {
            libc::setsockopt(
                tcp.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&raw const size).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
        let _ = tcp.set_nodelay(true);
    });
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("SMB loopback listener: {e}");
        }
    });
    tracing::debug!(%addr, "SMB loopback listener");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pace_spreads_pieces_out() {
        let pace = Pace::new(10 << 20);
        let inner = Pace::of(pace.id).unwrap();
        let t0 = Instant::now();
        for _ in 0..4 {
            inner.take(1 << 20).await;
        }
        // The 4th MiB goes out 0.3 s after the first.
        assert!(t0.elapsed() >= Duration::from_millis(290), "{:?}", t0.elapsed());
        assert_eq!(pace.sent(), 4 << 20);
        // Unpaced: at once.
        pace.set_rate(0);
        let t1 = Instant::now();
        for _ in 0..10 {
            inner.take(1 << 20).await;
        }
        assert!(t1.elapsed() < Duration::from_millis(50));
        let id = pace.id;
        drop(pace);
        assert!(Pace::of(id).is_none(), "the loopback refuses a reader whose pace is gone");
    }

    #[test]
    fn names() {
        assert_eq!(parent("smb://nas/Media/movies/a.mkv").as_deref(), Some("smb://nas/Media/movies"));
        assert_eq!(parent("smb://nas/Media/a.mkv").as_deref(), Some("smb://nas/Media"));
        assert_eq!(parent("smb://nas/Media"), None, "a share has no parent folder");
        assert_eq!(parent("/media/movies/a.mkv").as_deref(), Some("/media/movies"));
        assert_eq!(file_name("smb://nas/Media/movies/a b.mkv"), "a b.mkv");
        assert_eq!(join("smb://nas/M/", "x"), "smb://nas/M/x");
        assert_eq!(percent("a b(1).mkv"), "a%20b%281%29.mkv");
        assert_eq!(ffmpeg_input("/media/x.mkv"), "/media/x.mkv");
    }

    #[tokio::test]
    async fn local_listing_and_reads() {
        let dir = std::env::temp_dir().join(format!("moonrise-vfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("f.bin"), b"0123456789").unwrap();
        let d = dir.to_string_lossy().to_string();
        let mut l = read_dir(&d).await.unwrap();
        l.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            l.iter().map(|e| (e.name.as_str(), e.is_dir, e.is_file, e.size)).collect::<Vec<_>>(),
            vec![("f.bin", false, true, 10), ("sub", true, false, 0)]
        );
        let f = join(&d, "f.bin");
        assert_eq!(&read_at(&f, 3, 4).await.unwrap()[..], b"3456");
        assert_eq!(&read_at(&f, 8, 10).await.unwrap()[..], b"89");
        let s: Vec<Bytes> = stream(&f, 2, 5).await.unwrap().map(|b| b.unwrap()).collect().await;
        assert_eq!(s.concat(), b"23456");
        assert!(is_file(&f).await && !is_file(&d).await);
        assert_eq!(stat(&f).await.unwrap().size, 10);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
