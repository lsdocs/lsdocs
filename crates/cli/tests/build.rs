//! End-to-end builds through the `lsdocs` binary.
//!
//! The same site is built with its plugins in-process, out-of-process over
//! stdio, and out-of-process over a pipe; all three must produce identical
//! output. Out-of-process plugins are the builtins served by
//! `lsdocs plugin <name>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const LSDOCS: &str = env!("CARGO_BIN_EXE_lsdocs");
const PLUGINS: [(&str, &str); 6] = [
    ("markdown", "parser-markdown"),
    ("toc", "transform-toc"),
    ("links", "transform-links"),
    ("nav", "transform-nav"),
    ("highlight", "transform-highlight"),
    ("html", "render-html"),
];

/// A fresh, empty directory under the system temp dir.
fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lsdocs-test-{}-{label}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Writes the sample content into `site/docs`.
fn write_content(site: &Path) {
    let docs = site.join("docs");
    write(
        &docs.join("index.md"),
        "---\ntitle: Home\n---\n# Welcome\n\nHello & 1 < 2. Read [](guide/intro.md).\n\n## Install\n\nSteps.\n\n## Use\n\n```sh\nlsdocs build\n```\n",
    );
    write(
        &docs.join("guide/intro.md"),
        "# Intro\n\nOnly one heading, so no TOC. [Gone](missing.md).\n",
    );
    write(&docs.join("img/logo.svg"), "<svg/>");
    write(&docs.join(".hidden.md"), "# Not built\n");
}

/// A config running every plugin the given way: `None` for builtins, or
/// `Some("stdio" | "pipe")` for out-of-process.
fn config(transport: Option<&str>) -> String {
    let mut out =
        String::from("[site]\ntitle = \"Test Site\"\nbase_url = \"https://example.com/\"\n");
    for (name, builtin) in PLUGINS {
        out.push_str(&format!("\n[[plugins]]\nname = \"{name}\"\n"));
        match transport {
            None => out.push_str(&format!("builtin = \"{builtin}\"\n")),
            Some(transport) => {
                // Literal (single-quoted) TOML strings keep Windows paths intact.
                out.push_str(&format!(
                    "command = ['{LSDOCS}', 'plugin', '{builtin}']\ntransport = \"{transport}\"\n"
                ));
            }
        }
    }
    out
}

fn lsdocs(args: &[&str]) -> Output {
    Command::new(LSDOCS)
        .args(args)
        .env_remove("LSDOCS_LOG")
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Builds a site with the given plugin setup and returns its output files.
fn build_site(
    label: &str,
    transport: Option<&str>,
    extra_args: &[&str],
) -> (BTreeMap<String, Vec<u8>>, Output) {
    let site = temp_dir(label);
    write_content(&site);
    let config_path = site.join("lsdocs.toml");
    write(&config_path, &config(transport));
    let config_arg = config_path.to_string_lossy().into_owned();
    let mut args = vec!["build", "--config", config_arg.as_str()];
    args.extend_from_slice(extra_args);
    let output = lsdocs(&args);
    assert!(
        output.status.success(),
        "{label} build failed:\n{}",
        stderr(&output)
    );
    let files = read_tree(&site.join("site"));
    let _ = std::fs::remove_dir_all(&site);
    (files, output)
}

fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &rel, out);
            } else {
                out.insert(rel, std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, "", &mut out);
    out
}

#[test]
fn in_process_and_out_of_process_builds_match() {
    let (in_process, _) = build_site("builtin", None, &[]);
    let (stdio, _) = build_site("stdio", Some("stdio"), &[]);
    let (pipe, _) = build_site("pipe", Some("pipe"), &[]);

    let files: Vec<_> = in_process.keys().map(String::as_str).collect();
    assert_eq!(
        files,
        [
            "guide/intro.html",
            "img/logo.svg",
            "index.html",
            "sitemap.xml"
        ]
    );

    let index = String::from_utf8(in_process["index.html"].clone()).unwrap();
    assert!(index.contains("<title>Home | Test Site</title>"), "{index}");
    assert!(index.contains("Hello &amp; 1 &lt; 2."), "{index}");
    assert!(
        index.contains("<div class=\"toc\">"),
        "the TOC is missing:\n{index}"
    );
    assert!(
        index.contains("<a href=\"#install\">Install</a>"),
        "{index}"
    );
    // A cross-reference takes the linked page's title.
    assert!(
        index.contains("<a href=\"guide/intro.html\">Intro</a>"),
        "{index}"
    );
    // Navigation, and the sitemap from `render/site`.
    assert!(
        index.contains("<a href=\"index.html\" aria-current=\"page\">Home</a>"),
        "{index}"
    );
    let sitemap = String::from_utf8(in_process["sitemap.xml"].clone()).unwrap();
    assert!(
        sitemap.contains("<loc>https://example.com/guide/intro.html</loc>"),
        "{sitemap}"
    );
    let intro = String::from_utf8(in_process["guide/intro.html"].clone()).unwrap();
    assert!(
        !intro.contains("<div class=\"toc\">"),
        "single-heading pages get no TOC:\n{intro}"
    );

    assert_eq!(in_process, stdio, "stdio output differs from in-process");
    assert_eq!(in_process, pipe, "pipe output differs from in-process");
}

#[test]
fn the_cache_skips_unchanged_pages() {
    let site = temp_dir("cache");
    write_content(&site);
    let config_path = site.join("lsdocs.toml");
    write(&config_path, &config(None));
    let config_arg = config_path.to_string_lossy().into_owned();
    let build = |extra: &[&str]| {
        let mut args = vec!["build", "--config", config_arg.as_str()];
        args.extend_from_slice(extra);
        let output = lsdocs(&args);
        assert!(output.status.success(), "{}", stderr(&output));
        (read_tree(&site.join("site")), stderr(&output))
    };

    // How many results the build log says came from the cache.
    let hits = |log: &str| -> usize {
        log.split(" result(s) from the cache")
            .next()
            .filter(|s| s.len() < log.len())
            .and_then(|s| s.rsplit('(').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    let (first, log) = build(&[]);
    assert_eq!(hits(&log), 0, "{log}");
    assert!(site.join(".lsdocs-cache").is_dir());
    // Unchanged: every result comes from the cache, and the site is the same.
    let (second, log) = build(&[]);
    let all = hits(&log);
    assert!(all > 0, "{log}");
    assert_eq!(first, second);
    // Diagnostics in cached results are reported again.
    assert!(log.contains("warning[broken-link]"), "{log}");

    // A reworded page is processed again; the other page's results, whose
    // inputs did not change, still come from the cache.
    write(
        &site.join("docs/guide/intro.md"),
        "# Intro\n\nChanged. [Gone](missing.md).\n",
    );
    let (third, log) = build(&[]);
    assert!((1..all).contains(&hits(&log)), "{all} vs. {log}");
    let intro = String::from_utf8(third["guide/intro.html"].clone()).unwrap();
    assert!(intro.contains("Changed."), "{intro}");

    let (_, log) = build(&["--no-cache"]);
    assert_eq!(hits(&log), 0, "{log}");
    let _ = std::fs::remove_dir_all(&site);
}

#[test]
fn broken_links_are_warnings() {
    let (_, output) = build_site("broken-link", None, &[]);
    let log = stderr(&output);
    assert!(
        log.contains(
            "warning[broken-link]: guide/intro.md:3:30: link target `guide/missing.md` does not exist"
        ),
        "{log}"
    );
}

#[test]
fn trace_rpc_logs_messages_as_json() {
    let (_, output) = build_site("trace", Some("stdio"), &["--trace-rpc"]);
    let log = stderr(&output);
    assert!(
        log.contains(r#"[0,1,"initialize",[{"#),
        "no initialize request in:\n{log}"
    );
    assert!(
        log.contains(r#"[2,"exit",[]]"#),
        "no exit notification in:\n{log}"
    );
}

#[test]
fn init_then_build() {
    let dir = temp_dir("init");
    let dir_arg = dir.to_string_lossy().into_owned();
    let output = lsdocs(&["init", &dir_arg]);
    assert!(output.status.success(), "{}", stderr(&output));

    // Refuses to overwrite.
    let again = lsdocs(&["init", &dir_arg]);
    assert!(!again.status.success());
    assert!(
        stderr(&again).contains("already exists"),
        "{}",
        stderr(&again)
    );

    let config = dir.join("lsdocs.toml").to_string_lossy().into_owned();
    let output = lsdocs(&["build", "-c", &config, "--output-dir", "public"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(dir.join("public/index.html").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_errors_are_reported() {
    let dir = temp_dir("bad-config");
    write(&dir.join("docs/index.md"), "# Hi\n");
    let config = dir.join("lsdocs.toml");
    write(
        &config,
        "[[plugins]]\nname = \"x\"\nbuiltin = \"no-such-plugin\"\n",
    );
    let config_arg = config.to_string_lossy().into_owned();
    let output = lsdocs(&["build", "-c", &config_arg]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("plugin `x`: no builtin named `no-such-plugin` (available: parser-markdown, render-html, transform-highlight, transform-links, transform-nav, transform-toc)"),
        "{err}"
    );

    write(&config, "[site]\ntitel = \"typo\"\n");
    let output = lsdocs(&["build", "-c", &config_arg]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("titel"), "{}", stderr(&output));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_crashing_plugin_fails_the_build_cleanly() {
    let dir = temp_dir("crash");
    write(&dir.join("docs/index.md"), "# Hi\n");
    let config = dir.join("lsdocs.toml");
    // A "plugin" that exits immediately without speaking the protocol.
    write(
        &config,
        &format!("[[plugins]]\nname = \"broken\"\ncommand = ['{LSDOCS}', '--version']\n"),
    );
    let config_arg = config.to_string_lossy().into_owned();
    let output = lsdocs(&["build", "-c", &config_arg]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("plugin `broken`"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
