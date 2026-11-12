//! SMB shares read directly by the server (SMB2/3, the `smb2` crate: pure Rust, pipelined
//! reads), so a library can live on a NAS without mounting anything on the host.
//!
//! **Paths.** A file on a share is named `smb://host[:port]/share/dir/file.mkv` everywhere the
//! server keeps paths (the library's `files`, the probe cache, the scan state): `/`-separated,
//! relative to the share after it. The name says nothing of the credentials, so changing a
//! password changes no file's identity.
//!
//! **Connections.** One per (host, share, credentials), opened on first use and kept; the crate
//! re-dials and signs in again by itself after a drop (`auto_reconnect`). A share that can't be
//! reached isn't asked again for 5 s (callers fail at once meanwhile). Open file handles are
//! kept for a minute after their last read (an open is a round trip), at most 64.
//!
//! **Signing.** Sessions are signed only when the server requires it (`smb.signing = auto`, as
//! the Linux kernel's client does): a NAS signing every read answered at 18 MB/s, unsigned at
//! 88 MB/s. `always` signs every session. Encryption is used when the share or server asks for it.
//!
//! **Reads.** Through a block cache (256 KiB blocks, `MOONRISE_SMB_CACHE_MB`, default 384; least
//! recently used dropped): every ffmpeg run reads the container's header and index again, an
//! audio conversion reads the clusters a video cut just read; the mounts had the kernel's page
//! cache for that. Small reads away from the head and tail (subtitle blocks) go straight to the
//! share. [`Smb::stream`] reads ahead as the reader keeps reading (up to 4 MiB in flight) for
//! long sequential reads (ffmpeg's input, downloads). A read runs on its own, whether or not its
//! caller is still polling it ([`Smb::read_at`]). Credentials come from the settings (`smb.credentials`, referenced by the
//! library roots); a share without any is opened as guest.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, OnceLock, RwLock},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures_util::{stream::FuturesOrdered, Stream, StreamExt};
use serde::Serialize;
use smb2::{client::connection::Connection, ClientConfig, ErrorKind, FileReader, SmbClient, Tree};

/// Seconds between 1601-01-01 and 1970-01-01, in 100 ns ticks.
const EPOCH_TICKS: i64 = 116_444_736_000_000_000;
/// The largest single READ (the crate splits bigger ones into sequential READs; we split them
/// ourselves to keep them in flight together).
const PIECE: u64 = 1 << 20;
const HANDLE_IDLE: Duration = Duration::from_secs(60);
const MAX_HANDLES: usize = 64;
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(5);
/// The block cache's unit (one READ each).
const BLOCK: u64 = 256 << 10;
/// Reads at least this long, or in the file's head or tail, go through the block cache.
const DIRECT_BELOW: u64 = 128 << 10;
const HEAD_BYTES: u64 = 2 << 20;
const TAIL_BYTES: u64 = 4 << 20;

/// A file time (100 ns ticks since 1601) as nanoseconds since the Unix epoch, as the kernel's
/// CIFS client reports it (so a file's mtime is the same read either way).
pub fn filetime_ns(ticks: u64) -> i64 {
    (ticks as i64 - EPOCH_TICKS).saturating_mul(100)
}

// ---------------------------------------------------------------- paths

/// `smb://host[:port]/share/path`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SmbPath {
    /// As given (a name or an address, `:port` when not 445).
    pub host: String,
    pub share: String,
    /// Relative to the share, `/`-separated, no leading or trailing slash ("" = the share).
    pub path: String,
}

pub fn is_smb(path: &str) -> bool {
    path.len() > 6 && path[..6].eq_ignore_ascii_case("smb://")
}

impl SmbPath {
    pub fn parse(s: &str) -> Option<SmbPath> {
        if !is_smb(s) {
            return None;
        }
        let rest = &s[6..];
        let (host, rest) = rest.split_once('/').unwrap_or((rest, ""));
        let (share, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.trim();
        let share = share.trim();
        if host.is_empty() || share.is_empty() || host.contains(['\\', '@']) {
            return None;
        }
        let path = clean_rel(path)?;
        Some(SmbPath {
            host: host.to_string(),
            share: share.to_string(),
            path,
        })
    }

    pub fn new(host: &str, share: &str, path: &str) -> Option<SmbPath> {
        SmbPath::parse(&format!("smb://{}/{}/{}", host.trim(), share.trim().trim_matches('/'), path))
    }

    pub fn url(&self) -> String {
        if self.path.is_empty() {
            format!("smb://{}/{}", self.host, self.share)
        } else {
            format!("smb://{}/{}/{}", self.host, self.share, self.path)
        }
    }

    pub fn join(&self, name: &str) -> SmbPath {
        let path = if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", self.path)
        };
        SmbPath { path, ..self.clone() }
    }

    fn key(&self) -> (String, String) {
        (self.host.to_ascii_lowercase(), self.share.to_ascii_lowercase())
    }
}

/// `host` with the SMB port unless it names one.
pub fn addr_of(host: &str) -> String {
    match host.rsplit_once(':') {
        Some((_, p)) if p.parse::<u16>().is_ok() && !host.starts_with('[') => host.to_string(),
        _ => format!("{host}:445"),
    }
}

/// A share-relative path made clean: no empty, `.` or `..` parts, `/`-separated.
pub fn clean_rel(p: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in p.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            x => parts.push(x),
        }
    }
    Some(parts.join("/"))
}

// ---------------------------------------------------------------- credentials

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Creds {
    /// Empty: guest.
    pub username: String,
    pub password: String,
    pub domain: String,
}

impl std::fmt::Debug for Creds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Creds")
            .field("username", &self.username)
            .field("domain", &self.domain)
            .field("password", &if self.password.is_empty() { "<unset>" } else { "***" })
            .finish()
    }
}

impl Creds {
    pub fn guest() -> Creds {
        Creds::default()
    }

    pub fn is_guest(&self) -> bool {
        self.username.is_empty()
    }
}

// ---------------------------------------------------------------- errors

/// What went wrong, for the admin API and the scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// Nothing answered (no route, refused, timed out).
    Unreachable,
    /// The server wants an account (guest refused).
    AuthRequired,
    /// The account was refused (or answered with a guest session).
    WrongPassword,
    /// Signed in, but not allowed here.
    AccessDenied,
    /// No such share, folder or file.
    NotFound,
    /// Something else (protocol, unsupported).
    Other,
}

impl Problem {
    pub fn message(self) -> &'static str {
        match self {
            Problem::Unreachable => "the server can't be reached",
            Problem::AuthRequired => "the share needs a user name and password",
            Problem::WrongPassword => "the user name or password is wrong",
            Problem::AccessDenied => "this account may not open it",
            Problem::NotFound => "no such share or folder",
            Problem::Other => "the server answered with an error",
        }
    }
}

/// An SMB error, classified.
#[derive(Debug)]
pub struct SmbError {
    pub problem: Problem,
    pub detail: String,
}

impl std::fmt::Display for SmbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.problem.message(), self.detail)
    }
}

impl std::error::Error for SmbError {}

fn classify(e: &smb2::Error, guest: bool) -> Problem {
    match e.kind() {
        ErrorKind::ConnectionLost | ErrorKind::TimedOut => Problem::Unreachable,
        ErrorKind::AuthRequired if guest => Problem::AuthRequired,
        ErrorKind::AuthRequired | ErrorKind::GuestSessionRefused => Problem::WrongPassword,
        ErrorKind::SigningRequired if guest => Problem::AuthRequired,
        // A guest turned away from a share: it needs an account.
        ErrorKind::AccessDenied if guest => Problem::AuthRequired,
        ErrorKind::AccessDenied => Problem::AccessDenied,
        ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidName => Problem::NotFound,
        _ => Problem::Other,
    }
}

fn smb_err(e: smb2::Error, guest: bool, what: &str) -> anyhow::Error {
    smb_err_ref(&e, guest, what)
}

fn smb_err_ref(e: &smb2::Error, guest: bool, what: &str) -> anyhow::Error {
    anyhow::Error::new(SmbError {
        problem: classify(e, guest),
        detail: format!("{what}: {e}"),
    })
}

/// The classification of an error from this module (Other for anything else).
pub fn problem_of(e: &anyhow::Error) -> Problem {
    e.chain()
        .find_map(|c| c.downcast_ref::<SmbError>())
        .map_or(Problem::Other, |s| s.problem)
}

/// The error as an `io::Error` of the matching kind (the scanner tells "gone" from "can't look").
pub fn io_error(e: anyhow::Error) -> std::io::Error {
    use std::io::ErrorKind as K;
    let kind = match problem_of(&e) {
        Problem::NotFound => K::NotFound,
        Problem::AccessDenied | Problem::AuthRequired | Problem::WrongPassword => K::PermissionDenied,
        Problem::Unreachable => K::NotConnected,
        Problem::Other => K::Other,
    };
    std::io::Error::new(kind, format!("{e:#}"))
}

// ---------------------------------------------------------------- connections

/// A signed-in session with one share.
pub struct Share {
    /// Owns the connection (kept alive with it).
    _client: tokio::sync::Mutex<SmbClient>,
    tree: Arc<Tree>,
    conn: Connection,
    guest: bool,
}

impl Share {
    /// Connects and signs in (no pooling: for the admin API's one-off looks).
    pub async fn connect(host: &str, share: &str, creds: &Creds, sign_always: bool) -> Result<Share> {
        let mut client = connect_client(host, creds, sign_always).await?;
        let tree = client
            .connect_share(share)
            .await
            .map_err(|e| smb_err(e, creds.is_guest(), &format!("open share {share}")))?;
        let conn = client
            .connection_for(&tree)
            .map_err(|e| smb_err(e, creds.is_guest(), "connection"))?;
        Ok(Share {
            _client: tokio::sync::Mutex::new(client),
            tree: Arc::new(tree),
            conn,
            guest: creds.is_guest(),
        })
    }

    pub async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let mut conn = self.conn.clone();
        let entries = self
            .tree
            .list_directory(&mut conn, path)
            .await
            .map_err(|e| smb_err(e, self.guest, &format!("list {path:?}")))?;
        Ok(entries
            .into_iter()
            .filter(|e| e.name != "." && e.name != "..")
            .map(|e| Entry {
                is_link: e.is_link(),
                name: e.name,
                is_dir: e.is_directory,
                size: e.size,
                mtime: filetime_ns(e.modified.0),
            })
            .collect())
    }

    pub async fn stat(&self, path: &str) -> Result<Meta> {
        let mut conn = self.conn.clone();
        let i = self
            .tree
            .stat(&mut conn, path)
            .await
            .map_err(|e| smb_err(e, self.guest, &format!("stat {path:?}")))?;
        Ok(Meta {
            is_dir: i.is_directory,
            size: i.size,
            mtime: filetime_ns(i.modified.0),
        })
    }

    pub async fn open(&self, path: &str) -> Result<FileReader> {
        self.tree
            .open_file_reader(self.conn.clone(), path)
            .await
            .map_err(|e| smb_err(e, self.guest, &format!("open {path:?}")))
    }
}

async fn connect_client(host: &str, creds: &Creds, sign_always: bool) -> Result<SmbClient> {
    let config = ClientConfig {
        addr: addr_of(host),
        timeout: Duration::from_secs(5),
        username: creds.username.clone(),
        password: creds.password.clone(),
        domain: creds.domain.clone(),
        auto_reconnect: true,
        // Video doesn't compress; LZ4 would only cost CPU on both ends.
        compression: false,
        // Home shares aren't DFS; a mistyped share would otherwise be asked about on IPC$.
        dfs_enabled: false,
        sign_when_optional: sign_always,
        ..Default::default()
    };
    SmbClient::connect(config)
        .await
        .map_err(|e| smb_err(e, creds.is_guest(), &format!("connect to {host}")))
}

/// Lists the disk shares of a server (hidden `$` ones left out). Always signed: Samba turns an
/// unsigned session away from `IPC$` (and it costs nothing here).
pub async fn list_shares(host: &str, creds: &Creds, _sign_always: bool) -> Result<Vec<ShareInfo>> {
    let mut client = connect_client(host, creds, true).await?;
    let shares = client
        .list_shares()
        .await
        .map_err(|e| smb_err(e, creds.is_guest(), "list shares"))?;
    #[allow(
        clippy::verbose_bit_mask,
        reason = "the mask names the share type field; trailing_zeros would hide it"
    )]
    let mut out: Vec<ShareInfo> = shares
        .into_iter()
        // STYPE_DISKTREE = 0; the top bits are flags (special/temporary).
        .filter(|s| s.share_type & 0x0FFF_FFFF == 0 && !s.name.ends_with('$'))
        .map(|s| ShareInfo {
            name: s.name,
            comment: s.comment,
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareInfo {
    pub name: String,
    pub comment: String,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// A junction or symlink (not followed by walks).
    pub is_link: bool,
    pub size: u64,
    /// ns since the epoch.
    pub mtime: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Meta {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Default)]
struct Slot {
    share: tokio::sync::Mutex<Option<Arc<Share>>>,
    failed: Mutex<Option<(Instant, String, Problem)>>,
}

struct OpenFile {
    reader: Arc<FileReader>,
    used: Instant,
}

/// The server's SMB client: credentials per share, connections, open handles.
pub struct Smb {
    creds: RwLock<HashMap<(String, String), Creds>>,
    sign_always: std::sync::atomic::AtomicBool,
    slots: Mutex<HashMap<(String, String, Creds), Arc<Slot>>>,
    files: Mutex<HashMap<String, OpenFile>>,
    cache: BlockCache,
}

/// A file as the cache keys it: (url, size, mtime), so a changed file isn't served old bytes.
type FileId = Arc<(String, u64, i64)>;
/// How a block's read ended: its bytes, or the error everyone waiting for it gets.
type BlockRead = std::result::Result<Bytes, Arc<smb2::Error>>;
/// A block (read once, shared by everyone asking) and when it was last asked for.
type CacheSlot = (Arc<tokio::sync::OnceCell<BlockRead>>, u64);

/// Recently read 256 KiB blocks of shared files, least recently used dropped first: what the
/// kernel's page cache did for the mounts (every ffmpeg run reads the container's header and
/// index again; an audio conversion reads the clusters a video cut just read). A block being
/// read is read once for everyone asking, and if that read fails, everyone waiting for it fails
/// with it: else each would read it again in turn, one after another's timeout (half a minute each
/// with the share gone), while the reads before them hold the blocks.
struct BlockCache {
    map: Mutex<HashMap<(FileId, u64), CacheSlot>>,
    tick: std::sync::atomic::AtomicU64,
    bytes: std::sync::atomic::AtomicU64,
    cap: u64,
}

impl BlockCache {
    fn new(cap: u64) -> BlockCache {
        BlockCache {
            map: Mutex::new(HashMap::new()),
            tick: 0.into(),
            bytes: 0.into(),
            cap,
        }
    }

    async fn get<F, Fut>(&self, file: FileId, block: u64, fetch: F) -> BlockRead
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Bytes, smb2::Error>>,
    {
        use std::sync::atomic::Ordering::Relaxed;
        if self.cap == 0 {
            return fetch().await.map_err(Arc::new);
        }
        let key = (file, block);
        let tick = self.tick.fetch_add(1, Relaxed);
        let cell = {
            let mut m = self.map.lock().unwrap();
            let e = m
                .entry(key.clone())
                .or_insert_with(|| (Arc::new(tokio::sync::OnceCell::new()), tick));
            e.1 = tick;
            e.0.clone()
        };
        let mut fetched = false;
        let r = cell
            .get_or_init(|| {
                fetched = true;
                async { fetch().await.map_err(Arc::new) }
            })
            .await
            .clone();
        match &r {
            Ok(b) if fetched => {
                if self.bytes.fetch_add(b.len() as u64, Relaxed) + b.len() as u64 > self.cap {
                    self.evict();
                }
            }
            Ok(_) => {}
            // A failed read is forgotten: whoever asks next reads it again.
            Err(_) => {
                let mut m = self.map.lock().unwrap();
                if m.get(&key).is_some_and(|(c, _)| Arc::ptr_eq(c, &cell)) {
                    m.remove(&key);
                }
            }
        }
        r
    }

    /// Down to 90 % of the cap, oldest first.
    fn evict(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut m = self.map.lock().unwrap();
        let mut done: Vec<(u64, (FileId, u64), u64)> = m
            .iter()
            .filter_map(|(k, (c, t))| c.get().and_then(|r| r.as_ref().ok()).map(|b| (*t, k.clone(), b.len() as u64)))
            .collect();
        done.sort_by_key(|x| x.0);
        let mut total: u64 = done.iter().map(|x| x.2).sum();
        let target = self.cap / 10 * 9;
        for (_, k, n) in done {
            if total <= target {
                break;
            }
            m.remove(&k);
            total -= n;
        }
        self.bytes.store(total, Relaxed);
    }
}

static SMB: OnceLock<Arc<Smb>> = OnceLock::new();

/// The process's client (made on first use; the sweeper of idle handles starts with it).
pub fn client() -> Arc<Smb> {
    SMB.get_or_init(|| {
        let smb = Arc::new(Smb {
            creds: RwLock::new(HashMap::new()),
            sign_always: false.into(),
            slots: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
            cache: BlockCache::new(
                std::env::var("MOONRISE_SMB_CACHE_MB")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(384)
                    << 20,
            ),
        });
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let s = smb.clone();
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    s.files.lock().unwrap().retain(|_, f| f.used.elapsed() < HANDLE_IDLE);
                }
            });
        }
        smb
    })
    .clone()
}

/// A share the libraries use, with its credentials (None: guest).
#[derive(Clone, Debug, PartialEq)]
pub struct ShareConfig {
    pub host: String,
    pub share: String,
    pub creds: Option<Creds>,
}

impl Smb {
    /// The shares and credentials in effect (the settings changed); sessions with old
    /// credentials are dropped.
    pub fn configure(&self, shares: &[ShareConfig], sign_always: bool) {
        let map: HashMap<(String, String), Creds> = shares
            .iter()
            .map(|s| {
                (
                    (s.host.to_ascii_lowercase(), s.share.to_ascii_lowercase()),
                    s.creds.clone().unwrap_or_default(),
                )
            })
            .collect();
        let sign_changed = self.sign_always.swap(sign_always, std::sync::atomic::Ordering::Relaxed) != sign_always;
        let mut creds = self.creds.write().unwrap();
        if *creds != map || sign_changed {
            *creds = map;
            let keep: Vec<(String, String, Creds)> = if sign_changed {
                Vec::new()
            } else {
                creds.iter().map(|((h, s), c)| (h.clone(), s.clone(), c.clone())).collect()
            };
            self.slots.lock().unwrap().retain(|k, _| keep.contains(k));
            self.files.lock().unwrap().clear();
        }
    }

    pub fn sign_always(&self) -> bool {
        self.sign_always.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn creds_for(&self, p: &SmbPath) -> Creds {
        self.creds.read().unwrap().get(&p.key()).cloned().unwrap_or_default()
    }

    /// The pooled session for a path's share.
    pub async fn share(&self, p: &SmbPath) -> Result<Arc<Share>> {
        let creds = self.creds_for(p);
        let (h, s) = p.key();
        let slot = self.slots.lock().unwrap().entry((h, s, creds.clone())).or_default().clone();
        if let Some((at, why, problem)) = slot.failed.lock().unwrap().clone() {
            if at.elapsed() < RETRY_AFTER_FAILURE {
                return Err(anyhow::Error::new(SmbError { problem, detail: why }));
            }
        }
        let mut g = slot.share.lock().await;
        if let Some(s) = g.as_ref() {
            return Ok(s.clone());
        }
        match Share::connect(&p.host, &p.share, &creds, self.sign_always()).await {
            Ok(s) => {
                let s = Arc::new(s);
                *g = Some(s.clone());
                *slot.failed.lock().unwrap() = None;
                tracing::info!(host = %p.host, share = %p.share, guest = creds.is_guest(), "SMB share connected");
                Ok(s)
            }
            Err(e) => {
                *slot.failed.lock().unwrap() = Some((Instant::now(), format!("{e:#}"), problem_of(&e)));
                Err(e)
            }
        }
    }

    /// Forgets a share's session after an error that may mean it is dead (the next use dials
    /// again).
    fn drop_share(&self, p: &SmbPath) {
        let creds = self.creds_for(p);
        let (h, s) = p.key();
        if let Some(slot) = self.slots.lock().unwrap().get(&(h, s, creds)) {
            if let Ok(mut g) = slot.share.try_lock() {
                *g = None;
            }
        }
        let prefix = format!("{}/", p.url().to_ascii_lowercase());
        self.files
            .lock()
            .unwrap()
            .retain(|k, _| !k.to_ascii_lowercase().starts_with(&prefix) && !k.eq_ignore_ascii_case(&p.url()));
    }

    async fn retrying<T, F, Fut>(&self, p: &SmbPath, f: F) -> Result<T>
    where
        F: Fn(Arc<Share>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let share = self.share(p).await?;
        match f(share).await {
            Err(e) if problem_of(&e) == Problem::Unreachable => {
                // The session may be gone for good (the crate gave up re-dialing): a new one.
                let share_root = SmbPath {
                    path: String::new(),
                    ..p.clone()
                };
                self.drop_share(&share_root);
                let share = self.share(p).await?;
                f(share).await
            }
            r => r,
        }
    }

    pub async fn list(&self, p: &SmbPath) -> Result<Vec<Entry>> {
        let path = p.path.clone();
        self.retrying(p, |s| {
            let path = path.clone();
            async move { s.list(&path).await }
        })
        .await
    }

    pub async fn stat(&self, p: &SmbPath) -> Result<Meta> {
        let path = p.path.clone();
        self.retrying(p, |s| {
            let path = path.clone();
            async move { s.stat(&path).await }
        })
        .await
    }

    /// An open handle on the file (kept a while for the next read).
    pub async fn open(&self, p: &SmbPath) -> Result<Arc<FileReader>> {
        let url = p.url();
        if let Some(f) = self.files.lock().unwrap().get_mut(&url) {
            f.used = Instant::now();
            return Ok(f.reader.clone());
        }
        let path = p.path.clone();
        let reader = Arc::new(
            self.retrying(p, |s| {
                let path = path.clone();
                async move { s.open(&path).await }
            })
            .await?,
        );
        let mut files = self.files.lock().unwrap();
        if files.len() >= MAX_HANDLES {
            if let Some(oldest) = files.iter().min_by_key(|(_, f)| f.used).map(|(k, _)| k.clone()) {
                files.remove(&oldest);
            }
        }
        files.insert(
            url,
            OpenFile {
                reader: reader.clone(),
                used: Instant::now(),
            },
        );
        Ok(reader)
    }

    fn forget_handle(&self, p: &SmbPath) {
        self.files.lock().unwrap().remove(&p.url());
    }

    /// Size and mtime of a file as its open handle saw them.
    pub async fn file_meta(&self, p: &SmbPath) -> Result<Meta> {
        let r = self.open(p).await?;
        Ok(Meta {
            is_dir: r.info().is_directory,
            size: r.size(),
            mtime: filetime_ns(r.info().modified.0),
        })
    }

    /// `len` bytes at `offset` (fewer at the end of the file): 1 MiB pieces, all in flight.
    ///
    /// Runs as a task of its own, so it finishes whatever becomes of the caller. A read holds
    /// things others wait for: the blocks of the cache it is reading, the share's session while
    /// it dials. And a caller may keep a read in flight and not poll it for a long time: a
    /// stream's read-ahead isn't polled while its reader takes no bytes (a paused ffmpeg, a full
    /// pipe, a slow client), a subtitle job's reads aren't while it waits for its turn. Others
    /// would wait for as long, however little they want: a cut on the file of a paused extraction
    /// would wait for the pause to end, which waits for the cut to end.
    pub async fn read_at(self: &Arc<Self>, p: &SmbPath, offset: u64, len: u64) -> Result<Bytes> {
        let (smb, p) = (self.clone(), p.clone());
        tokio::spawn(async move { smb.read_retrying(&p, offset, len).await })
            .await
            .context("SMB read")?
    }

    async fn read_retrying(&self, p: &SmbPath, offset: u64, len: u64) -> Result<Bytes> {
        match self.read_once(p, offset, len).await {
            // A handle from before a reconnect is dead: open it again, once.
            Err(e) if problem_of(&e) == Problem::Unreachable => {
                self.forget_handle(p);
                self.read_once(p, offset, len).await
            }
            r => r,
        }
    }

    async fn read_once(&self, p: &SmbPath, offset: u64, len: u64) -> Result<Bytes> {
        let r = self.open(p).await?;
        let guest = self.creds_for(p).is_guest();
        let size = r.size();
        let end = (offset + len).min(size);
        if offset >= end {
            return Ok(Bytes::new());
        }
        let err = |e: smb2::Error| smb_err(e, guest, &format!("read {}", p.url()));
        let shared_err = |e: Arc<smb2::Error>| smb_err_ref(&e, guest, &format!("read {}", p.url()));
        // Small scattered reads (subtitle blocks) go straight to the share; the rest through the
        // block cache, as do the file's head and tail (container headers and indexes, read again
        // by every ffmpeg run).
        let in_head_or_tail = offset < HEAD_BYTES || end + TAIL_BYTES > size;
        if end - offset < DIRECT_BELOW && !in_head_or_tail {
            return r.read_at(offset, end - offset).await.map(Bytes::from).map_err(err);
        }
        let file = Arc::new((p.url(), size, filetime_ns(r.info().modified.0)));
        let (b0, b1) = (offset / BLOCK, (end - 1) / BLOCK);
        let blocks: Vec<Bytes> = futures_util::stream::iter(b0..=b1)
            .map(|b| {
                let r = r.clone();
                let file = file.clone();
                async move {
                    self.cache
                        .get(file, b, || async move {
                            let at = b * BLOCK;
                            r.read_at(at, BLOCK.min(size - at)).await.map(Bytes::from)
                        })
                        .await
                }
            })
            .buffered(8)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, Arc<smb2::Error>>>()
            .map_err(shared_err)?;
        let from = (offset - b0 * BLOCK) as usize;
        let n = (end - offset) as usize;
        if blocks.len() == 1 {
            let b = &blocks[0];
            return Ok(b.slice(from.min(b.len())..(from + n).min(b.len())));
        }
        let mut out = Vec::with_capacity(n);
        for b in &blocks {
            out.extend_from_slice(b);
        }
        let to = (from + n).min(out.len());
        Ok(Bytes::from(out).slice(from.min(to)..to))
    }

    /// The bytes [start, start+len) as a stream, read ahead: the first pieces small (a reader
    /// that only wants a header shouldn't cost megabytes), then 1 MiB, up to 4 in flight.
    pub fn stream(self: &Arc<Self>, p: SmbPath, start: u64, len: u64) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
        let smb = self.clone();
        let end = start + len;
        let stats = StreamStats {
            url: p.url(),
            start,
            t0: Instant::now(),
            issued: 0,
            sent: 0,
        };
        futures_util::stream::unfold(
            (FuturesOrdered::new(), start, 0usize, false, stats),
            move |(mut inflight, mut next, mut issued, mut done, mut stats): (
                FuturesOrdered<futures_util::future::BoxFuture<'static, Result<Bytes>>>,
                u64,
                usize,
                bool,
                StreamStats,
            )| {
                let smb = smb.clone();
                let p = p.clone();
                async move {
                    if done {
                        return None;
                    }
                    // Read ahead as the reader shows it wants more: one piece at first, two
                    // once it took 512 KiB, four after 2 MiB.
                    let window = match stats.sent {
                        0..524_288 => 1,
                        524_288..2_097_152 => 2,
                        _ => 4,
                    };
                    while inflight.len() < window && next < end {
                        let size = if issued < 2 { BLOCK } else { PIECE };
                        let n = size.min(end - next);
                        let (smb, p, at) = (smb.clone(), p.clone(), next);
                        inflight.push_back(Box::pin(async move { smb.read_at(&p, at, n).await })
                            as futures_util::future::BoxFuture<'static, Result<Bytes>>);
                        next += n;
                        issued += 1;
                        stats.issued += n;
                    }
                    match inflight.next().await {
                        None => None,
                        Some(Ok(b)) if b.is_empty() => None,
                        Some(Ok(b)) => {
                            stats.sent += b.len() as u64;
                            Some((Ok(b), (inflight, next, issued, done, stats)))
                        }
                        Some(Err(e)) => {
                            done = true;
                            Some((Err(io_error(e)), (inflight, next, issued, done, stats)))
                        }
                    }
                }
            },
        )
    }
}

/// What a stream read, logged (debug) when it ends.
struct StreamStats {
    url: String,
    start: u64,
    t0: Instant,
    issued: u64,
    sent: u64,
}

impl Drop for StreamStats {
    fn drop(&mut self) {
        tracing::debug!(url = %self.url, start = self.start, sent = self.sent, read = self.issued, ms = self.t0.elapsed().as_millis() as u64, "SMB stream");
    }
}

// ---------------------------------------------------------------- discovery

/// A file server seen on the local network.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Found {
    /// Its name (Bonjour or NetBIOS).
    pub name: String,
    /// What to connect to (an address).
    pub host: String,
    pub addresses: Vec<String>,
    /// "mdns" and/or "netbios".
    pub via: Vec<&'static str>,
}

/// Looks for SMB servers for `wait`: Bonjour (`_smb._tcp`) and a NetBIOS node-status
/// broadcast (Windows, most NAS). Merged by address.
pub async fn discover(wait: Duration) -> Vec<Found> {
    let (m, n) = tokio::join!(tokio::task::spawn_blocking(move || discover_mdns(wait)), discover_netbios(wait));
    let mut out: Vec<Found> = Vec::new();
    for f in m.unwrap_or_default().into_iter().chain(n) {
        match out.iter_mut().find(|o| o.addresses.iter().any(|a| f.addresses.contains(a))) {
            Some(o) => {
                for v in f.via {
                    if !o.via.contains(&v) {
                        o.via.push(v);
                    }
                }
                for a in f.addresses {
                    if !o.addresses.contains(&a) {
                        o.addresses.push(a);
                    }
                }
            }
            None => out.push(f),
        }
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

fn discover_mdns(wait: Duration) -> Vec<Found> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    let Ok(daemon) = ServiceDaemon::new() else { return Vec::new() };
    let Ok(rx) = daemon.browse("_smb._tcp.local.") else {
        let _ = daemon.shutdown();
        return Vec::new();
    };
    let deadline = Instant::now() + wait;
    let mut out: Vec<Found> = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let mut addrs: Vec<String> = info
                    .get_addresses()
                    .iter()
                    .filter(|a| a.is_ipv4())
                    .map(ToString::to_string)
                    .collect();
                addrs.sort();
                let Some(first) = addrs.first().cloned() else { continue };
                let name = info.get_fullname().trim_end_matches("._smb._tcp.local.").to_string();
                let host = if info.get_port() == 445 {
                    first
                } else {
                    format!("{first}:{}", info.get_port())
                };
                if !out.iter().any(|o| o.host == host) {
                    out.push(Found {
                        name,
                        host,
                        addresses: addrs,
                        via: vec!["mdns"],
                    });
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse("_smb._tcp.local.");
    let _ = daemon.shutdown();
    out
}

/// A NetBIOS node-status request for `*` (RFC 1002 4.2.17), the way `nmblookup -S '*'` asks.
pub fn nbstat_request(id: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(50);
    p.extend_from_slice(&id.to_be_bytes());
    // Flags 0 (a node status query is not a broadcast query), 1 question.
    p.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    p.push(32);
    let mut name = [0u8; 16];
    name[0] = b'*';
    for b in name {
        p.push(b'A' + (b >> 4));
        p.push(b'A' + (b & 0x0f));
    }
    p.push(0);
    p.extend_from_slice(&[0x00, 0x21, 0x00, 0x01]);
    p
}

/// The file-server name (suffix 0x20, unique) in a node-status answer.
pub fn nbstat_name(resp: &[u8], id: u16) -> Option<String> {
    if resp.len() < 57 || resp[0..2] != id.to_be_bytes() || resp[2] & 0x80 == 0 {
        return None;
    }
    // Header 12, encoded name 34, type/class 4, TTL 4, rdlength 2, then the name count.
    let mut i = 12 + 34 + 4 + 4 + 2;
    let count = *resp.get(i)? as usize;
    i += 1;
    let mut fallback = None;
    for _ in 0..count {
        let rec = resp.get(i..i + 18)?;
        let name = String::from_utf8_lossy(&rec[..15]).trim_end().to_string();
        let suffix = rec[15];
        let group = rec[16] & 0x80 != 0;
        if !group && suffix == 0x20 {
            return Some(name);
        }
        if !group && suffix == 0x00 && fallback.is_none() {
            fallback = Some(name);
        }
        i += 18;
    }
    fallback
}

async fn discover_netbios(wait: Duration) -> Vec<Found> {
    let Ok(sock) = tokio::net::UdpSocket::bind("0.0.0.0:0").await else {
        return Vec::new();
    };
    let _ = sock.set_broadcast(true);
    let id: u16 = rand::random();
    let req = nbstat_request(id);
    for b in broadcast_addresses() {
        let _ = sock.send_to(&req, SocketAddr::new(IpAddr::V4(b), 137)).await;
    }
    let mut out: Vec<Found> = Vec::new();
    let deadline = tokio::time::Instant::now() + wait;
    let mut buf = vec![0u8; 2048];
    loop {
        let r = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await;
        let Ok(Ok((n, from))) = r else { break };
        if let Some(name) = nbstat_name(&buf[..n], id) {
            let ip = from.ip().to_string();
            if !out.iter().any(|o| o.host == ip) {
                out.push(Found {
                    name,
                    host: ip.clone(),
                    addresses: vec![ip],
                    via: vec!["netbios"],
                });
            }
        }
    }
    out
}

/// The IPv4 broadcast addresses of the LAN interfaces (Docker bridges and VPNs left out).
fn broadcast_addresses() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !crate::mdns::is_virtual(&i.name))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if v4.ip.is_private() => Some(v4.broadcast.unwrap_or_else(|| {
                let m = u32::from(v4.netmask);
                Ipv4Addr::from(u32::from(v4.ip) | !m)
            })),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A host the admin gave: a name or an address, optional `:port`, nothing else.
pub fn check_host(host: &str) -> Result<String> {
    let h = host
        .trim()
        .trim_start_matches("smb://")
        .trim_start_matches("\\\\")
        .trim_end_matches(['/', '\\']);
    let ok = !h.is_empty() && h.len() <= 255 && h.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c));
    if !ok {
        return Err(anyhow!("{host:?}: a server name or address"));
    }
    Ok(h.to_string())
}

/// A share name the admin gave.
pub fn check_share(share: &str) -> Result<String> {
    let s = share.trim().trim_matches(['/', '\\']);
    if s.is_empty() || s.len() > 80 || s.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)) {
        return Err(anyhow!("{share:?}: a share name"));
    }
    Ok(s.to_string())
}

/// Tests one root: signs in, lists the folder; the folder must exist and hold something.
pub async fn probe_root(p: &SmbPath, creds: &Creds, sign_always: bool) -> Result<(Vec<Entry>, Duration)> {
    let t0 = Instant::now();
    let share = Share::connect(&p.host, &p.share, creds, sign_always).await?;
    let entries = share.list(&p.path).await.with_context(|| p.url())?;
    Ok((entries, t0.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everyone waiting for a block that fails gets the error at once; nobody reads it again
    /// after another timeout. The next to ask does read it again.
    #[tokio::test]
    async fn a_failed_block_read_fails_everyone_waiting() {
        let cache = Arc::new(BlockCache::new(1 << 30));
        let file: FileId = Arc::new(("smb://h/s/f".into(), 1 << 20, 0));
        let t0 = Instant::now();
        let holder = {
            let (cache, file) = (cache.clone(), file.clone());
            tokio::spawn(async move {
                cache
                    .get(file, 0, || async {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        Err(smb2::Error::Timeout)
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let read_again = std::sync::atomic::AtomicBool::new(false);
        let waiter = cache
            .get(file.clone(), 0, || async {
                read_again.store(true, std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(Bytes::from_static(b"late"))
            })
            .await;
        assert!(waiter.is_err());
        assert!(!read_again.load(std::sync::atomic::Ordering::Relaxed), "the waiter read it again");
        assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
        assert!(holder.await.unwrap().is_err());
        let next = cache.get(file, 0, || async { Ok(Bytes::from_static(b"ok")) }).await.unwrap();
        assert_eq!(&next[..], b"ok");
    }

    #[test]
    fn paths() {
        let p = SmbPath::parse("smb://nas.local/Media/movies/A (2000)/a.mkv").unwrap();
        assert_eq!(
            (p.host.as_str(), p.share.as_str(), p.path.as_str()),
            ("nas.local", "Media", "movies/A (2000)/a.mkv")
        );
        assert_eq!(p.url(), "smb://nas.local/Media/movies/A (2000)/a.mkv");
        assert_eq!(addr_of(&p.host), "nas.local:445");
        let r = SmbPath::parse("SMB://10.0.0.2:1445/share/").unwrap();
        assert_eq!(
            (addr_of(&r.host).as_str(), r.path.as_str(), r.url().as_str()),
            ("10.0.0.2:1445", "", "smb://10.0.0.2:1445/share")
        );
        assert_eq!(r.join("x").url(), "smb://10.0.0.2:1445/share/x");
        assert_eq!(
            SmbPath::new("nas", "/Media/", "//tv shows//").unwrap().url(),
            "smb://nas/Media/tv shows"
        );
        assert!(SmbPath::parse("smb://nas/").is_none());
        assert!(SmbPath::parse("smb://nas/share/../x").is_none());
        assert!(SmbPath::parse("/media/x").is_none());
        assert!(is_smb("smb://a/b") && !is_smb("/smb://"));
        // The parent of a file is a plain string operation (Path works on these too).
        let parent = std::path::Path::new("smb://nas/Media/movies/a.mkv")
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(parent, "smb://nas/Media/movies");
    }

    #[test]
    fn filetimes() {
        // 2024-01-09 (A.Real.Pain on the NAS): the kernel's CIFS client said the same nanoseconds.
        assert_eq!(filetime_ns(116_444_736_000_000_000), 0);
        assert_eq!(filetime_ns(116_444_736_000_000_001), 100);
        assert_eq!(filetime_ns(134_124_286_558_614_348), 1_767_955_055_861_434_800);
    }

    #[test]
    fn hosts_and_shares() {
        assert_eq!(check_host(" smb://nas.local/ ").unwrap(), "nas.local");
        assert_eq!(check_host("\\\\NAS").unwrap(), "NAS");
        assert_eq!(check_host("192.168.1.2:1445").unwrap(), "192.168.1.2:1445");
        assert!(check_host("nas; rm").is_err());
        assert!(check_host("").is_err());
        assert_eq!(check_share("/Media/").unwrap(), "Media");
        assert!(check_share("a/b").is_err());
    }

    #[test]
    fn netbios_packets() {
        let req = nbstat_request(0x1234);
        assert_eq!(req.len(), 50);
        assert_eq!(&req[..2], &[0x12, 0x34]);
        assert_eq!(&req[13..15], b"CK", "'*' encodes as CK");
        assert_eq!(&req[46..], &[0, 0x21, 0, 1]);
        // An answer with a group name, then the server's file-server name.
        let mut resp = vec![0x12, 0x34, 0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0];
        resp.extend_from_slice(&req[12..46]);
        resp.extend_from_slice(&[0, 0x21, 0, 1, 0, 0, 0, 0, 0, 0]);
        resp.push(2);
        let rec = |name: &str, suffix: u8, flags: u8| {
            let mut r = format!("{name:<15}").into_bytes();
            r.push(suffix);
            r.extend_from_slice(&[flags, 0]);
            r
        };
        resp.extend(rec("WORKGROUP", 0x00, 0x80));
        resp.extend(rec("NAS", 0x20, 0x04));
        assert_eq!(nbstat_name(&resp, 0x1234).as_deref(), Some("NAS"));
        assert_eq!(nbstat_name(&resp, 0x9999), None, "another id");
    }

    #[test]
    fn creds_never_print_the_password() {
        let c = Creds {
            username: "u".into(),
            password: "hunter2".into(),
            domain: String::new(),
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
