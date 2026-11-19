//! Drives `lsdocs serve` over real HTTP: pages are served with the reload
//! script, and edits, new files and deleted files are rebuilt and announced
//! on the reload stream.

// These tests build sites with the built-in plugins.
#![cfg(all(
    feature = "extras",
    feature = "parser-markdown",
    feature = "parser-rst",
    feature = "render-html",
    feature = "sphinx-addons",
    feature = "transform-highlight",
    feature = "transform-links",
    feature = "transform-nav",
    feature = "transform-toc"
))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const LSDOCS: &str = env!("CARGO_BIN_EXE_lsdocs");

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Kills the server when the test ends, even on failure.
struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start() -> Server {
    let dir = std::env::temp_dir().join(format!("lsdocs-serve-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(
        &dir.join("lsdocs.toml"),
        "[site]\ntitle = \"Served\"\n\n[[plugins]]\nname = \"markdown\"\nbuiltin = \"parser-markdown\"\n\n\
         [[plugins]]\nname = \"links\"\nbuiltin = \"transform-links\"\n\n\
         [[plugins]]\nname = \"html\"\nbuiltin = \"render-html\"\n",
    );
    write(&dir.join("docs/index.md"), "# Home\n\nFirst version.\n");
    let mut child = Command::new(LSDOCS)
        .arg("serve")
        .arg("--config")
        .arg(dir.join("lsdocs.toml"))
        .args(["--port", "0"])
        .env_remove("LSDOCS_LOG")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read the log on a thread; the first "serving" line has the port.
    let (tx, rx) = mpsc::channel();
    let stderr = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            // Echoed, so a failing test shows the server's log.
            eprintln!("[serve] {line}");
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let port = loop {
        let line = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the server did not start");
        if let Some(rest) = line.split("http://127.0.0.1:").nth(1) {
            break rest
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .split('/')
                .next()
                .unwrap()
                .parse()
                .unwrap();
        }
    };
    Server { child, port, dir }
}

/// GETs `path`; returns the status code and body.
fn get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1.to_owned();
    (status, body)
}

/// A connection to the reload stream.
struct Reloads(BufReader<TcpStream>);

impl Reloads {
    fn open(port: u16) -> Self {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /__lsdocs/reload HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while !line.starts_with(": connected") {
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        Self(reader)
    }

    /// Waits for the next reload event.
    fn next(&mut self) {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .0
                .read_line(&mut line)
                .expect("no reload event in time");
            assert!(n > 0, "the reload stream closed");
            if line.trim() == "data: reload" {
                return;
            }
        }
    }
}

/// Polling and rebuilds take a moment; also lets file times differ.
fn settle() {
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn serve_rebuilds_and_reloads() {
    let server = start();
    let port = server.port;

    let (status, body) = get(port, "/");
    assert_eq!(status, 200);
    assert!(body.contains("First version."), "{body}");
    assert!(
        body.contains("new EventSource(\"/__lsdocs/reload\")"),
        "{body}"
    );
    assert_eq!(get(port, "/missing.html").0, 404);
    assert_eq!(get(port, "/../lsdocs.toml").0, 404);

    let mut reloads = Reloads::open(port);

    // An edit is rebuilt and announced.
    settle();
    write(
        &server.dir.join("docs/index.md"),
        "# Home\n\nSecond version.\n",
    );
    reloads.next();
    assert!(get(port, "/").1.contains("Second version."));

    // A new file restarts the plugins with the new file list, so links to
    // it check out.
    write(
        &server.dir.join("docs/new.md"),
        "# New\n\nBack [home](index.md).\n",
    );
    reloads.next();
    let (status, body) = get(port, "/new.html");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("<a href=\"index.html\">home</a>"), "{body}");

    // A deleted file's page is removed.
    std::fs::remove_file(server.dir.join("docs/new.md")).unwrap();
    reloads.next();
    assert_eq!(get(port, "/new.html").0, 404);
    assert!(!server.dir.join("site/new.html").exists());

    // A plugin's settings file under `.config/` restarts the plugins.
    assert!(get(port, "/").1.contains("class=\"lsd-anchor\""));
    write(
        &server.dir.join(".config/lsdocs/html/settings.toml"),
        "heading_anchors = false\n",
    );
    reloads.next();
    let (_, body) = get(port, "/");
    assert!(!body.contains("class=\"lsd-anchor\""), "{body}");
}
