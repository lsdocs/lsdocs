//! `lsdocs serve`: build, serve the output over HTTP, and rebuild when
//! files change, reloading open pages.
//!
//! - **Watching** polls the site directory every [`POLL`] for changed
//!   modification times and sizes. Polling needs no platform support and
//!   also works on network and WSL mounts, where file events often do not
//!   arrive. The output directory, the cache and hidden directories are
//!   ignored.
//! - **Rebuilding** reuses one [`Session`], so plugins stay running and
//!   cached results stay in memory. A change to the config file starts a
//!   new session.
//! - **Serving** is a minimal HTTP/1.1 server for `GET` and `HEAD` of files
//!   in the output directory, for local development only. HTML responses
//!   get a small script that listens on [`RELOAD_PATH`] (server-sent
//!   events) and reloads the page after a rebuild changes any output.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lsdocs_config::{Config, Overrides};
use lsdocs_host::{BuildOptions, BuildReport, Builtins, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// How often the site directory is checked for changes.
pub const POLL: Duration = Duration::from_millis(250);

/// The server-sent events endpoint pages listen on for reloads.
pub const RELOAD_PATH: &str = "/__lsdocs/reload";

/// Injected before `</body>` in served HTML pages.
const RELOAD_SCRIPT: &str =
    "<script>new EventSource(\"/__lsdocs/reload\").onmessage = () => location.reload();</script>\n";

/// Where to serve from.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Address to listen on.
    pub addr: SocketAddr,
    /// Build options, e.g. whether to use the cache.
    pub build: BuildOptions,
}

/// Runs `lsdocs serve` until interrupted.
pub async fn serve(
    config_path: &Path,
    overrides: &Overrides,
    builtins: &Builtins,
    options: &ServeOptions,
) -> Result<(), String> {
    let root = crate::site_root(config_path);
    let load = || crate::load_config(config_path, overrides.clone());
    let mut session = Session::start(load()?, &root, builtins, &options.build)
        .await
        .map_err(|e| e.to_string())?;
    let output_dir = root.join(&session.config().output_dir);
    rebuild(&mut session).await;

    let listener = TcpListener::bind(options.addr)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", options.addr))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (reload, _) = broadcast::channel(16);
    tokio::spawn(http(listener, output_dir.clone(), reload.clone()));
    eprintln!(
        "serving {} at http://{addr}/ (Ctrl+C to stop)",
        output_dir.display()
    );

    let mut watched = Snapshot::take(
        &root,
        &ignored(&root, session.config()),
        session.dependencies(),
    );
    let config_dir = lsdocs_host::config_dir(&root);
    let mut config_state = ConfigState::take(config_path, &config_dir);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            () = tokio::time::sleep(POLL) => {}
        }
        let ignore = ignored(&root, session.config());
        let now = Snapshot::take(&root, &ignore, session.dependencies());
        if now == watched && ConfigState::take(config_path, &config_dir) == config_state {
            continue;
        }
        // Let a burst of saves settle before rebuilding.
        tokio::time::sleep(POLL).await;
        watched = Snapshot::take(&root, &ignore, session.dependencies());
        let config_now = ConfigState::take(config_path, &config_dir);
        let changed = if config_now != config_state {
            config_state = config_now;
            eprintln!("config changed; restarting plugins");
            match load() {
                Ok(config) => match Session::start(config, &root, builtins, &options.build).await {
                    Ok(new) => {
                        std::mem::replace(&mut session, new).stop().await;
                        rebuild(&mut session).await
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        false
                    }
                },
                Err(e) => {
                    eprintln!("error: {e}");
                    false
                }
            }
        } else {
            rebuild(&mut session).await
        };
        if changed {
            let _ = reload.send(());
        }
    }
    session.stop().await;
    Ok(())
}

/// Builds and reports; returns whether any output changed.
async fn rebuild(session: &mut Session) -> bool {
    let started = std::time::Instant::now();
    match session.build().await {
        Ok(report) => {
            print_report(&report);
            eprintln!(
                "rebuilt in {} ms: {} file(s) changed{}",
                started.elapsed().as_millis(),
                report.changed.len(),
                match report.cached {
                    0 => String::new(),
                    n => format!(", {n} result(s) from the cache"),
                }
            );
            !report.changed.is_empty()
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

fn print_report(report: &BuildReport) {
    for d in &report.diagnostics {
        eprintln!("{}", report.format_diagnostic(d));
    }
}

/// Directories the watcher skips: the output directory and the cache.
/// Hidden entries are skipped too.
fn ignored(root: &Path, config: &Config) -> Vec<PathBuf> {
    vec![
        root.join(&config.output_dir),
        root.join(lsdocs_host::CACHE_DIR),
    ]
}

/// A file's modification time and size, if it exists.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Modification times and sizes of every watched file: those under the
/// site directory, and the files the last build depended on (which may be
/// elsewhere, such as the code of generated API pages; a missing one counts
/// as watched too, so creating it rebuilds).
#[derive(Debug, PartialEq, Eq)]
struct Snapshot(BTreeMap<PathBuf, (SystemTime, u64)>);

impl Snapshot {
    fn take(root: &Path, ignore: &[PathBuf], extra: &BTreeSet<PathBuf>) -> Self {
        fn walk(dir: &Path, ignore: &[PathBuf], out: &mut BTreeMap<PathBuf, (SystemTime, u64)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if hidden || ignore.iter().any(|i| path.starts_with(i)) {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => walk(&path, ignore, out),
                    Ok(_) => {
                        if let Some(stamp) = stamp(&path) {
                            out.insert(path, stamp);
                        }
                    }
                    Err(_) => {}
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(root, ignore, &mut files);
        for path in extra {
            let stamp = stamp(path).unwrap_or((SystemTime::UNIX_EPOCH, u64::MAX));
            files.entry(path.clone()).or_insert(stamp);
        }
        Self(files)
    }
}

/// The site config and every plugin's settings under `.config/`: a change
/// to either restarts the plugins, which read their settings at startup.
#[derive(Debug, PartialEq, Eq)]
struct ConfigState(Option<(SystemTime, u64)>, Snapshot);

impl ConfigState {
    fn take(config_path: &Path, config_dir: &Path) -> Self {
        Self(
            stamp(config_path),
            Snapshot::take(config_dir, &[], &BTreeSet::new()),
        )
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// Accepts connections forever, serving files from `dir`.
async fn http(listener: TcpListener, dir: PathBuf, reload: broadcast::Sender<()>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let dir = dir.clone();
        let reload = reload.subscribe();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, &dir, reload).await {
                tracing::debug!("http: {e}");
            }
        });
    }
}

/// A parsed request line.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    method: String,
    path: String,
}

/// Parses the request line of an HTTP/1.x request head.
fn parse_request(head: &str) -> Option<Request> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    parts.next().filter(|v| v.starts_with("HTTP/1."))?;
    let path = target.split(['?', '#']).next().unwrap_or("/");
    Some(Request {
        method,
        path: lsdocs_plugin_sdk::links::percent_decode(path),
    })
}

/// The file under `dir` that `path` names: `/` and directories mean their
/// `index.html`. `None` for paths that try to leave `dir`.
fn resolve(dir: &Path, path: &str) -> Option<PathBuf> {
    let mut file = dir.to_path_buf();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." || part.contains(['\\', ':']) {
            return None;
        }
        file.push(part);
    }
    if file.is_dir() {
        file.push("index.html");
    }
    Some(file)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "md" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// `html` with the reload script added before its last `</body>`, or at
/// the end if it has none.
fn inject(html: &[u8]) -> Vec<u8> {
    let at = html
        .windows(7)
        .rposition(|w| w.eq_ignore_ascii_case(b"</body>"))
        .unwrap_or(html.len());
    let mut out = Vec::with_capacity(html.len() + RELOAD_SCRIPT.len());
    out.extend_from_slice(&html[..at]);
    out.extend_from_slice(RELOAD_SCRIPT.as_bytes());
    out.extend_from_slice(&html[at..]);
    out
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    stream.shutdown().await
}

async fn handle(
    mut stream: TcpStream,
    dir: &Path,
    mut reload: broadcast::Receiver<()>,
) -> std::io::Result<()> {
    // Read the request head (the body of a GET is ignored).
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > 16 * 1024 {
            return respond(
                &mut stream,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"",
                false,
            )
            .await;
        }
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf))
            .await
            .map_err(|_| std::io::Error::other("timed out reading the request"))??;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let Some(request) = parse_request(&String::from_utf8_lossy(&head)) else {
        return respond(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"bad request\n",
            false,
        )
        .await;
    };
    let head_only = request.method == "HEAD";
    if request.method != "GET" && !head_only {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"",
            false,
        )
        .await;
    }
    if request.path == RELOAD_PATH {
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Cache-Control: no-store\r\nConnection: keep-alive\r\n\r\n: connected\n\n",
            )
            .await?;
        loop {
            match reload.recv().await {
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    stream.write_all(b"data: reload\n\n").await?;
                }
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            }
        }
    }
    let file = resolve(dir, &request.path).filter(|f| f.is_file());
    let Some(file) = file else {
        return respond(
            &mut stream,
            "404 Not Found",
            "text/html; charset=utf-8",
            &inject(b"<!doctype html><title>Not found</title><h1>Not found</h1>\n"),
            head_only,
        )
        .await;
    };
    let body = tokio::fs::read(&file).await?;
    let content_type = content_type(&file);
    let body = if content_type.starts_with("text/html") {
        inject(&body)
    } else {
        body
    };
    respond(&mut stream, "200 OK", content_type, &body, head_only).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn request_lines() {
        assert_eq!(
            parse_request("GET /guide/a%20b.html?x=1 HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(Request {
                method: "GET".into(),
                path: "/guide/a b.html".into()
            })
        );
        assert_eq!(parse_request("GET /\r\n"), None);
        assert_eq!(parse_request("nonsense"), None);
    }

    #[test]
    fn paths_stay_in_the_output_directory() {
        let dir = std::env::temp_dir().join(format!("lsdocs-serve-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("guide")).unwrap();
        assert_eq!(resolve(&dir, "/"), Some(dir.join("index.html")));
        assert_eq!(
            resolve(&dir, "/guide/"),
            Some(dir.join("guide").join("index.html"))
        );
        assert_eq!(
            resolve(&dir, "/a/b.html"),
            Some(dir.join("a").join("b.html"))
        );
        assert_eq!(resolve(&dir, "/../secret"), None);
        assert_eq!(resolve(&dir, "/a/..\\..\\x"), None);
        assert_eq!(resolve(&dir, "/C:/x"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_reload_script_goes_before_the_body_end() {
        let out = String::from_utf8(inject(b"<body><p>hi</p></BODY></html>")).unwrap();
        assert!(out.starts_with("<body><p>hi</p><script>"), "{out}");
        assert!(out.ends_with("</script>\n</BODY></html>"), "{out}");
        let bare = String::from_utf8(inject(b"<p>x</p>")).unwrap();
        assert!(bare.starts_with("<p>x</p><script>"), "{bare}");
    }

    #[test]
    fn snapshots_see_changes_and_skip_ignored_dirs() {
        let dir = std::env::temp_dir().join(format!("lsdocs-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join("site")).unwrap();
        std::fs::write(dir.join("docs/a.md"), "a").unwrap();
        let ignore = [dir.join("site")];
        let none = BTreeSet::new();
        let before = Snapshot::take(&dir, &ignore, &none);
        std::fs::write(dir.join("site/out.html"), "ignored").unwrap();
        std::fs::write(dir.join(".hidden"), "ignored").unwrap();
        assert_eq!(Snapshot::take(&dir, &ignore, &none), before);
        std::fs::write(dir.join("docs/a.md"), "a longer text").unwrap();
        assert_ne!(Snapshot::take(&dir, &ignore, &none), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
