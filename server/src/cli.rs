//! The command line (`main.rs` calls [`run`]):
//!
//! ```text
//! moonrise-server                              serve (the default)
//! moonrise-server scan [--full]                a scan pass in the running server; prints its report
//! moonrise-server stats [--recent <hours>]     counts, identification states, usage
//! moonrise-server llm-test                     one identification call, to check the LLM setup
//! moonrise-server llm-replay [<n>]             asks the configured LLM the last <n> (20) questions
//!                                          kept in the database and compares its answers with the
//!                                          ones kept (another provider or model; nothing written)
//! moonrise-server user add <name> [--admin]    a Moonrise account; the password is read from stdin (the
//!                                          first admin claims a new server, like the app's setup)
//! moonrise-server user passwd <name>           a new password, from stdin
//! moonrise-server user list                    accounts and their tokens (devices)
//! moonrise-server user revoke <name>           revoke all of a user's tokens
//! moonrise-server user delete <name> --yes     delete an account (tokens, watch state go with it)
//! moonrise-server subs-test <file> [--scan] [--out <dir>] [--parallel <n>]
//!                                          read a Matroska file's text subtitles directly (timing)
//! moonrise-server migrate-root <from> <to> [--credential <id>] [--apply]
//!                                          move a library root (mount ↔ SMB share) keeping
//!                                          everything known about its files (server stopped)
//! moonrise-server smb-credential add --user <name> [--host <host>] [--domain <d>]
//!                                          store SMB credentials (password from stdin) in the
//!                                          running server; prints the id
//! moonrise-server users reset-data [--user <name>] --yes
//!                                          forget watch state, sessions, tokens, device codes
//! ```

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tracing_subscriber::EnvFilter;

use crate::{
    admin::{config, logbuf, manage, settings},
    api, auth, db, downloads, library,
    llm::{self, chatgpt},
    media::{self, credits, hwaccel, index_backfill, mkv, trickplay},
    migrate,
    storage::{cache, source},
    subtitles::{subs, subx},
};

/// The server's command line: `serve` (the default) or one of the subcommands above.
pub async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map_or("serve", String::as_str);
    if matches!(cmd, "--version" | "-V" | "version") {
        println!("moonrise-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cli = cmd != "serve";
    init_logging(cli);
    let env = config::Env::from_process();
    let config = config::Config::resolve(&env, &settings::Stored::default());
    db::init_prober(&config.ffprobe);
    match cmd {
        "serve" => serve(env).await,
        "healthcheck" => healthcheck(&config),
        "scan" => scan_command(&config, &args).await,
        "stats" => stats_command(&config, &args).await,
        "llm-test" => llm_test(config, &env).await,
        "llm-replay" => llm_replay(&config, &env, &args).await,
        "identify-eval" => identify_eval(&config, &env, &args).await,
        "align-eval" | "align-parts" => align_eval(&config, &env, &args).await,
        "user" | "users" => user_command(&config, &args[1..]),
        "subs-test" => subs_test(&args[1..]).await,
        "keyframes-test" => keyframes_test(&args[1..]).await,
        "credits-test" => credits_test(&config, &args[1..]).await,
        "migrate-root" => migrate_root(&config, &args),
        "merge-libraries" => merge_libraries(&config, &args),
        "smb-credential" => smb_credential(&config, &args).await,
        other => bail!("unknown command {other}"),
    }
}

/// Logs to stderr and the last lines in memory: warnings for the subcommands, more for `serve`
/// (`RUST_LOG` overrides).
fn init_logging(cli: bool) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            if cli {
                "moonrise_server=warn".into()
            } else {
                "moonrise_server=info,tower_http=warn".into()
            }
        }))
        // stderr, and the last lines in memory (the apps' log view).
        .with_writer(logbuf::make)
        // Plain text in `docker logs` (colours only on a terminal).
        .with_ansi(std::env::var_os("MOONRISE_LOG_FILE").is_none() && std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
}

/// The image's HEALTHCHECK: `GET /health` on the port the server listens on.
fn healthcheck(config: &config::Config) -> Result<()> {
    use std::io::{Read, Write};
    let port = config.listen.port();
    let mut s = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), std::time::Duration::from_secs(3))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(4)))?;
    s.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    let mut r = String::new();
    let _ = s.read_to_string(&mut r);
    if r.starts_with("HTTP/1.0 200") || r.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        bail!("unhealthy: {}", r.lines().next().unwrap_or("no answer"))
    }
}

/// `scan [--full]`: a scan pass in the running server; prints its report.
async fn scan_command(config: &config::Config, args: &[String]) -> Result<()> {
    let full = args.iter().any(|a| a == "--full");
    let v = local(config, reqwest::Method::POST, &format!("/v1/admin/scan?full={}", u8::from(full))).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// `stats [--recent <hours>]`: the running server's counts, identification states, usage.
async fn stats_command(config: &config::Config, args: &[String]) -> Result<()> {
    let recent = args
        .iter()
        .position(|a| a == "--recent")
        .and_then(|i| args.get(i + 1))
        .map(|h| format!("?recent={h}"))
        .unwrap_or_default();
    let v = local(config, reqwest::Method::GET, &format!("/v1/admin/stats{recent}")).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// `llm-test`: one identification call, to check the LLM setup.
async fn llm_test(config: config::Config, env: &config::Env) -> Result<()> {
    // With the settings store, as the server would.
    let config = match db::Db::open(&config.data_dir.join("moonrise.db")) {
        Ok(db) => {
            let e = env.clone();
            let stored = db.call(move |c| settings::load(c, &e)).await?;
            chatgpt::init(db);
            config::Config::resolve(env, &stored)
        }
        Err(_) => config,
    };
    let llm = llm::Llm::new(config.llm.clone());
    if !config.llm.enabled {
        bail!("the LLM isn't configured (no key, or MOONRISE_LLM_ENABLED=off)");
    }
    let (input, _) = llm::admin::sample();
    let t0 = std::time::Instant::now();
    let v = llm
        .classify(library::identify::SYSTEM, &input.to_string(), &library::identify::verdict_schema())
        .await
        .map_err(|e| anyhow::anyhow!(llm.scrub(&format!("{e:#}"))))?;
    println!(
        "{}",
        serde_json::json!({"llm": llm.describe(), "output": v.output, "verdict": v.value, "input_tokens": v.input_tokens, "output_tokens": v.output_tokens, "ms": t0.elapsed().as_millis() as u64})
    );
    Ok(())
}

/// `llm-replay [<n>]`: the last questions kept in the database asked again; the answers compared.
async fn llm_replay(config: &config::Config, env: &config::Env, args: &[String]) -> Result<()> {
    let n: i64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20);
    let db = db::Db::open(&config.data_dir.join("moonrise.db"))?;
    let e = env.clone();
    let stored = db.call(move |c| settings::load(c, &e)).await?;
    chatgpt::init(db.clone());
    let config = config::Config::resolve(env, &stored);
    if !config.llm.enabled {
        bail!("the LLM isn't configured (no key or model, or MOONRISE_LLM_ENABLED=off)");
    }
    let llm = llm::Llm::new(config.llm.clone());
    let rows: Vec<(String, String, String)> = db
        .call(move |c| {
            let mut st =
                c.prepare("SELECT model, request, verdict FROM llm_verdicts WHERE verdict IS NOT NULL ORDER BY created_at DESC LIMIT ?1")?;
            let r = st
                .query_map([n], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(r)
        })
        .await?;
    let schema = library::identify::verdict_schema();
    let (mut same, mut failed) = (0, 0);
    for (model, request, kept) in &rows {
        let kept: serde_json::Value = serde_json::from_str(kept).unwrap_or_default();
        let req: serde_json::Value = serde_json::from_str(request).unwrap_or_default();
        let t0 = std::time::Instant::now();
        let r = llm.classify(library::identify::SYSTEM, request, &schema).await;
        let ms = t0.elapsed().as_millis() as u64;
        let pick = |v: &serde_json::Value| serde_json::json!({"kind": v["kind"], "tmdb_id": v["tmdb_id"], "season": v["season"], "episode": v["episode"], "confidence": v["confidence"]});
        let line = match r {
            Ok(v) => {
                let agree =
                    v.value["tmdb_id"] == kept["tmdb_id"] && v.value["kind"] == kept["kind"] && v.value["episode"] == kept["episode"];
                same += usize::from(agree);
                serde_json::json!({"path": req["path"], "kept": {"model": model, "verdict": pick(&kept)}, "now": pick(&v.value), "same": agree, "output": v.output, "ms": ms})
            }
            Err(e) => {
                failed += 1;
                serde_json::json!({"path": req["path"], "kept": {"model": model, "verdict": pick(&kept)}, "error": llm.scrub(&format!("{e:#}")), "ms": ms})
            }
        };
        println!("{line}");
    }
    println!(
        "{}",
        serde_json::json!({"llm": llm.describe(), "asked": rows.len(), "same": same, "failed": failed})
    );
    Ok(())
}

/// `identify-eval [--typed]`: what identifying every file again would change.
async fn identify_eval(config: &config::Config, env: &config::Env, args: &[String]) -> Result<()> {
    // On a copy of the database: TMDB answers are cached into it.
    let db = db::Db::open(&config.data_dir.join("moonrise.db"))?;
    let stored = db.call(|c| settings::Stored::load(c)).await?;
    let config = config::Config::resolve(env, &stored);
    let mut off = config.disabled.get();
    off.insert("llm".into());
    config.disabled.set(off);
    let cache = Arc::new(cache::Cache::new(std::env::temp_dir().join("moonrise-eval-cache"), 1 << 30));
    let lib = library::Library::new(config.clone(), db, cache);
    let v = lib.eval_blind(args.iter().any(|a| a == "--typed")).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// `align-eval` / `align-parts [--apply]`: aligning every season with TMDB's episode lists.
async fn align_eval(config: &config::Config, env: &config::Env, args: &[String]) -> Result<()> {
    // align-eval: on a copy of the database, what aligning every season with TMDB's
    // episode lists would change (TMDB answers are cached into it). align-parts --apply
    // makes the changes (server stopped).
    let apply = args[0] == "align-parts" && args.iter().any(|a| a == "--apply");
    let db = db::Db::open(&config.data_dir.join("moonrise.db"))?;
    let stored = db.call(|c| settings::Stored::load(c)).await?;
    let config = config::Config::resolve(env, &stored);
    let cache = Arc::new(cache::Cache::new(std::env::temp_dir().join("moonrise-eval-cache"), 1 << 30));
    let lib = library::Library::new(config.clone(), db, cache);
    let v = lib.align_parts(None, apply).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// `migrate-root <from> <to> [--credential <id>] [--apply]`: a library root moved.
fn migrate_root(config: &config::Config, args: &[String]) -> Result<()> {
    let pos: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();
    let cred = args.iter().position(|a| a == "--credential").and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = pos.into_iter().filter(|a| Some(*a) != cred.as_ref()).collect();
    let (Some(from), Some(to)) = (pos.first(), pos.get(1)) else {
        bail!("migrate-root <from> <to> [--credential <id>] [--apply]")
    };
    let apply = args.iter().any(|a| a == "--apply");
    let r = migrate::migrate(
        &config.data_dir.join("moonrise.db"),
        &config.data_dir.join("subs"),
        from,
        to,
        cred.as_deref(),
        apply,
    )?;
    println!("{}", serde_json::to_string_pretty(&r)?);
    if !apply {
        eprintln!("(a dry run: --apply to change it; stop the server first)");
    }
    Ok(())
}

/// One-off (undocumented): the production server's two typed libraries made one.
fn merge_libraries(config: &config::Config, args: &[String]) -> Result<()> {
    let name = args.iter().position(|a| a == "--name").and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = args[1..]
        .iter()
        .filter(|a| !a.starts_with("--") && Some(*a) != name.as_ref())
        .collect();
    let (Some(into), Some(from)) = (pos.first(), pos.get(1)) else {
        bail!("merge-libraries <into> <from> [--name <name>] [--apply]")
    };
    anyhow::ensure!(
        std::env::var_os("MOONRISE_LIBRARIES").is_none(),
        "MOONRISE_LIBRARIES is set: the libraries come from the environment"
    );
    let apply = args.iter().any(|a| a == "--apply");
    let r = migrate::merge_libraries(&config.data_dir.join("moonrise.db"), into, from, name.as_deref(), apply)?;
    println!("{}", serde_json::to_string_pretty(&r)?);
    if !apply {
        eprintln!("(a dry run: --apply to change it; stop the server first)");
    }
    Ok(())
}

/// `smb-credential add --user <name> [--host <host>] [--domain <d>]`: SMB credentials stored.
async fn smb_credential(config: &config::Config, args: &[String]) -> Result<()> {
    let opt = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    anyhow::ensure!(
        args.get(1).map(String::as_str) == Some("add"),
        "smb-credential add --user <name> [--host <host>] [--domain <d>]"
    );
    let user = opt("--user").context("--user <name>")?;
    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    let password = password.trim_end_matches(['\n', '\r']).to_string();
    let mut body = serde_json::json!({"username": user, "password": password});
    if let Some(h) = opt("--host") {
        body["host"] = h.into();
    }
    if let Some(d) = opt("--domain") {
        body["domain"] = d.into();
    }
    let v = local_with(config, reqwest::Method::POST, "/v1/admin/smb/credentials", Some(body)).await?;
    println!("{}", v["id"].as_str().unwrap_or(""));
    Ok(())
}

/// Reads Matroska files' keyframe indexes from their Cues, as the server would, one JSON line
/// per file (`fnv`: a hash of the times, to compare two builds). Files from the arguments, or one
/// path per line on stdin.
async fn keyframes_test(args: &[String]) -> Result<()> {
    let files: Vec<String> = if args.is_empty() {
        std::io::stdin().lines().map_while(Result::ok).collect()
    } else {
        args.to_vec()
    };
    let http = reqwest::Client::new();
    for path in files {
        let source = source::Source::Local(path.clone().into());
        let line = match mkv::keyframes(&source, &http).await {
            Ok(kf) => {
                let text: Vec<String> = kf.iter().map(|k| format!("{:016x}", k.to_bits())).collect();
                serde_json::json!({"file": path, "ok": true, "count": kf.len(), "first": kf.first(), "last": kf.last(), "fnv": format!("{:016x}", media::fnv(&text.join(",")))})
            }
            Err(e) => serde_json::json!({"file": path, "ok": false, "failure": format!("{:?}", mkv::failure(&e)), "err": format!("{e:#}")}),
        };
        println!("{line}");
    }
    Ok(())
}

/// Looks for the end credits in local files as the server would (`credits.rs`), one JSON line
/// per file (also reads a list on stdin). `--frames` adds the keyframes' darkness.
async fn credits_test(config: &config::Config, args: &[String]) -> Result<()> {
    let frames_too = args.iter().any(|a| a == "--frames");
    let args: Vec<String> = args.iter().filter(|a| *a != "--frames").cloned().collect();
    let files: Vec<String> = if args.is_empty() {
        std::io::stdin().lines().map_while(Result::ok).collect()
    } else {
        args
    };
    for path in files {
        let line = async {
            let out = tokio::process::Command::new(&config.ffprobe).args(["-v", "error", "-show_format", "-show_streams", "-of", "json", &path]).output().await?;
            let probe: serde_json::Value = serde_json::from_slice(&out.stdout)?;
            let duration: f64 = probe["format"]["duration"].as_str().and_then(|d| d.parse().ok()).context("no duration")?;
            let (_, stream, _) = media::index_plan(&probe, config).context("no video")?;
            let t0 = std::time::Instant::now();
            let start: f64 = probe["format"]["start_time"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
            let frames = credits::measure(&config.ffmpeg, &source::Source::Local(path.clone().into()), stream, duration, start).await?;
            let mut v = serde_json::json!({"file": path, "duration": duration, "start": credits::find(&frames, duration), "keyframes": frames.len(), "secs": t0.elapsed().as_secs_f64()});
            if frames_too {
                v["frames"] = serde_json::json!(frames);
            }
            anyhow::Ok(v)
        }
        .await
        .unwrap_or_else(|e| serde_json::json!({"file": path, "err": format!("{e:#}")}));
        println!("{line}");
    }
    Ok(())
}

/// Extracts a Matroska file's text tracks directly, as the server would, and says how long it
/// took (`--scan`: by scanning even if the index lists the blocks).
async fn subs_test(args: &[String]) -> Result<()> {
    let path = args.first().context("subs-test <file>")?;
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let parallel: usize = opt("--parallel").and_then(|p| p.parse().ok()).unwrap_or(8);
    let t0 = std::time::Instant::now();
    let source = source::Source::Local(path.into());
    let index = mkv::index(&source, &reqwest::Client::new()).await?;
    let streams: Vec<i64> = index
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == 17 && t.codec.starts_with("S_TEXT"))
        .map(|(i, _)| i as i64)
        .collect();
    let wants = subx::wants(&index, &streams).context("no text tracks")?;
    let method = if args.iter().any(|a| a == "--scan") {
        subx::Method::Scan
    } else {
        subx::method(&index, &wants)
    };
    let file = subx::File::open(std::path::Path::new(path), method == subx::Method::Cues)?;
    let index_ms = t0.elapsed().as_millis();
    let pace = move || -> futures_util::future::BoxFuture<'static, Result<subx::Step>> {
        Box::pin(async move {
            Ok(subx::Step {
                parallel,
                rest: std::time::Duration::ZERO,
                rate: 0,
            })
        })
    };
    let (m, cues) = subx::extract_with(&file, &index, &wants, &pace, method).await?;
    let counts: Vec<(i64, usize)> = wants.iter().map(|w| (w.stream, cues.get(&w.stream).map_or(0, Vec::len))).collect();
    println!(
        "{}",
        serde_json::json!({"file": path, "method": format!("{m:?}"), "index_ms": index_ms, "secs": t0.elapsed().as_secs_f64(), "cues": counts})
    );
    if let Some(out) = opt("--out") {
        std::fs::create_dir_all(&out)?;
        for w in &wants {
            let srt = subs::normalize_srt(&subx::to_srt(cues.get(&w.stream).map_or(&[][..], Vec::as_slice)));
            std::fs::write(std::path::Path::new(&out).join(format!("s{}.srt", w.stream)), srt)?;
        }
    }
    Ok(())
}

async fn serve(env: config::Env) -> Result<()> {
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");
    let state = Arc::new(api::AppState::new(env).await?);
    let config = state.config.clone();
    tokio::spawn(cache::Cache::janitor(state.cache.clone()));
    tokio::spawn(downloads::Downloads::janitor(state.downloads.clone()));
    // What transcodes video (NVIDIA, Intel/AMD, the CPU), before anything asks.
    {
        let c = config.clone();
        let hw = tokio::task::spawn_blocking(move || hwaccel::detect(&c.ffmpeg, &c.hwaccel, &c.hwaccel_device, c.transcode_jobs)).await?;
        hwaccel::set(hw);
    }
    if config.enabled(config::Feature::Transcode) && hwaccel::current().backend == hwaccel::Backend::Nvenc {
        let t = state.transcoder.clone();
        tokio::spawn(async move { t.warm_up().await });
    }
    if let Some(l) = &state.library {
        l.spawn();
        tokio::spawn(api::subtitle_backfill(state.clone()));
        tokio::spawn(index_backfill::run(state.clone()));
        tokio::spawn(trickplay::run(state.clone()));
    }
    // Bonjour (`_moonrise._tcp`), re-announced when the name changes.
    {
        let st = state.clone();
        tokio::task::spawn_blocking(move || st.announce()).await?;
    }
    let app = api::router(state);
    manage::mark_started();
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    // The client address: sign-in attempts are rate-limited per address.
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
    Ok(())
}

/// A request to the running server's admin API, with the local admin token.
async fn local(config: &config::Config, method: reqwest::Method, path: &str) -> Result<serde_json::Value> {
    local_with(config, method, path, None).await
}

async fn local_with(
    config: &config::Config,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let token =
        std::fs::read_to_string(config.data_dir.join("secrets").join("admin.token")).context("no admin token (is the server running?)")?;
    let port = config.listen.port();
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()?
        .request(method, format!("http://127.0.0.1:{port}{path}"))
        .header("x-moonrise-admin", token.trim());
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.context("server unreachable")?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text)?)
}

type DbJob = Box<dyn FnOnce(&mut rusqlite::Connection) -> Result<serde_json::Value> + Send>;

/// `moonrise-server user …` / `users …`: accounts, straight in the database (the running server
/// sees changes at once, except a cached token, which lasts at most a minute).
fn user_command(config: &config::Config, args: &[String]) -> Result<()> {
    let db = db::Db::open(&config.data_dir.join("moonrise.db"))?;
    let sub = args.first().map_or("", String::as_str);
    let name = || args.get(1).filter(|n| !n.starts_with("--")).cloned().context("give the user name");
    let flag = |f: &str| args.iter().any(|a| a == f);
    let run = |f: DbJob| -> Result<()> {
        let v = tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(db.call(f)))?;
        println!("{}", serde_json::to_string_pretty(&v)?);
        Ok(())
    };
    match sub {
        "add" => {
            let (n, admin) = (name()?, flag("--admin"));
            let pw = password_from_stdin()?;
            run(Box::new(move |c| Ok(serde_json::json!(auth::add_user(c, &n, &pw, admin)?))))
        }
        "passwd" => {
            let n = name()?;
            let pw = password_from_stdin()?;
            run(Box::new(move |c| {
                auth::set_password(c, &n, &pw)?;
                Ok(serde_json::json!({"user": n, "password": "changed"}))
            }))
        }
        "list" => run(Box::new(|c| {
            let mut st = c.prepare(
                "SELECT u.name, u.is_admin, u.created_at, (SELECT COUNT(*) FROM tokens t WHERE t.user_id = u.id AND t.revoked_at IS NULL),
                        (SELECT group_concat(COALESCE(t.device_name, '?'), ', ') FROM tokens t WHERE t.user_id = u.id AND t.revoked_at IS NULL),
                        (SELECT COUNT(*) FROM user_data d WHERE d.user_id = u.id)
                 FROM users u ORDER BY u.name",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(serde_json::json!({"name": r.get::<_, String>(0)?, "is_admin": r.get::<_, bool>(1)?, "created_at": library::rfc3339(r.get(2)?),
                        "tokens": r.get::<_, i64>(3)?, "devices": r.get::<_, Option<String>>(4)?, "user_data_rows": r.get::<_, i64>(5)?}))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::json!(rows))
        })),
        "revoke" => {
            let n = name()?;
            run(Box::new(move |c| {
                let k = c.execute(
                    "UPDATE tokens SET revoked_at = ?2 WHERE revoked_at IS NULL AND user_id = (SELECT id FROM users WHERE name = ?1)",
                    rusqlite::params![n, library::util::now()],
                )?;
                Ok(serde_json::json!({"user": n, "revoked": k}))
            }))
        }
        "delete" => {
            let n = name()?;
            if !flag("--yes") {
                bail!("this deletes the account with its tokens and watch state: add --yes");
            }
            run(Box::new(move |c| {
                let k = c.execute("DELETE FROM users WHERE name = ?1", [&n])?;
                Ok(serde_json::json!({"user": n, "deleted": k}))
            }))
        }
        "reset-data" => {
            if !flag("--yes") {
                bail!("this forgets watch state, playback sessions, tokens and device codes (users stay): add --yes");
            }
            let user = args.iter().position(|a| a == "--user").and_then(|i| args.get(i + 1)).cloned();
            run(Box::new(move |c| auth::reset_data(c, user.as_deref())))
        }
        _ => bail!("user add <name> [--admin] | user passwd <name> | user list | user revoke <name> | user delete <name> --yes | users reset-data [--user <name>] --yes"),
    }
}

/// The first line of stdin (never echoed or logged).
fn password_from_stdin() -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    let pw = s.trim_end_matches(['\r', '\n']).to_string();
    if pw.is_empty() {
        bail!("give the password on stdin");
    }
    Ok(pw)
}
