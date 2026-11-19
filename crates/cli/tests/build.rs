//! End-to-end builds through the `lsdocs` binary.
//!
//! The same site is built with its plugins in-process, out-of-process over
//! stdio, and out-of-process over a pipe; all three must produce identical
//! output. Out-of-process plugins are the builtins served by
//! `lsdocs plugin <name>`.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

const LSDOCS: &str = env!("CARGO_BIN_EXE_lsdocs");
const PLUGINS: [(&str, &str); 8] = [
    ("markdown", "parser-markdown"),
    ("sphinx", "sphinx-addons"),
    ("extras", "extras"),
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
        "# Intro\n\nOnly one heading, so no TOC. [Gone](missing.md). {index}`Intro page`.\n",
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
    // The inline list is off by default; these tests look for it.
    write(
        &site.join(".config/lsdocs/toc/settings.toml"),
        "inline = true\n",
    );
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
fn generated_pages_have_no_edit_link() {
    // genindex.html is generated (sphinx-addons' `parse/transform/all`):
    // the host marks it `generated`, so the default theme links no source
    // file to edit.
    let site = temp_dir("generated");
    write_content(&site);
    let config_path = site.join("lsdocs.toml");
    write(&config_path, &config(None));
    write(
        &site.join(".config/lsdocs/html/settings.toml"),
        "theme_settings = { edit_url = \"https://example.com/edit/{path}\" }
",
    );
    let config_arg = config_path.to_string_lossy().into_owned();
    let output = lsdocs(&["build", "--config", config_arg.as_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let files = read_tree(&site.join("site"));
    let _ = std::fs::remove_dir_all(&site);
    let page = |path: &str| String::from_utf8(files[path].clone()).unwrap();
    let index = page("index.html");
    assert!(
        index.contains("href=\"https://example.com/edit/index.md\""),
        "{index}"
    );
    let genindex = page("genindex.html");
    assert!(genindex.contains("lsd-article"), "{genindex}");
    assert!(!genindex.contains("lsd-edit"), "{genindex}");
}

#[test]
fn in_process_and_out_of_process_builds_match() {
    let (in_process, _) = build_site("builtin", None, &[]);
    let (stdio, _) = build_site("stdio", Some("stdio"), &[]);
    let (pipe, _) = build_site("pipe", Some("pipe"), &[]);

    // The theme's assets, whose names carry hashes, are listed apart.
    let (assets, files): (Vec<_>, Vec<_>) = in_process
        .keys()
        .map(String::as_str)
        .partition(|path| path.starts_with("_lsdocs/"));
    assert!(
        assets.iter().any(|path| path.ends_with(".css")),
        "{assets:?}"
    );
    assert_eq!(
        files,
        [
            "genindex.html",
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
        index.contains("<div class=\"lsd-toc\">"),
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
    // Navigation, and the sitemap from `render/transform/all`.
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
        !intro.contains("<div class=\"lsd-toc\">"),
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
    // The search index still lists the cached page (its entry is kept
    // with its cached render result) and the changed page's new text.
    let index: serde_json::Value = serde_json::from_slice(&third["_lsdocs/search.json"]).unwrap();
    let pages: Vec<_> = index["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["url"].as_str().unwrap())
        .collect();
    assert_eq!(pages, ["guide/intro.html", "index.html"]);
    assert_eq!(index["pages"][1]["sections"][1]["title"], "Install");
    assert!(
        index["pages"][0]["sections"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Changed."),
        "{index}"
    );

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
        err.contains("plugin `x`: no builtin named `no-such-plugin` (available: apidoc-c, apidoc-cpp, apidoc-python, apidoc-rust, apidoc-typescript, extras, parser-markdown, parser-rst, render-html, sphinx-addons, transform-highlight, transform-links, transform-nav, transform-toc)"),
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

#[test]
fn config_file_changes_invalidate_cached_results() {
    let site = temp_dir("config");
    write(&site.join("docs/page.md"), "# Page\n\n## One\n\n### Two\n");
    let config_path = site.join("lsdocs.toml");
    write(
        &config_path,
        "[[plugins]]\nname = \"markdown\"\nbuiltin = \"parser-markdown\"\n\n\
         [[plugins]]\nname = \"toc\"\nbuiltin = \"transform-toc\"\n\n\
         [[plugins]]\nname = \"html\"\nbuiltin = \"render-html\"\n",
    );
    let config_arg = config_path.to_string_lossy().into_owned();
    let build = || {
        let output = lsdocs(&["build", "--config", config_arg.as_str()]);
        assert!(output.status.success(), "{}", stderr(&output));
        std::fs::read_to_string(site.join("site/page.html")).unwrap()
    };
    // The inline toc is off by default; with it on, it lists `Two` (depth 3,
    // the default).
    let toc = |page: &str| {
        page.split("<div class=\"lsd-toc\">")
            .nth(1)
            .and_then(|list| list.split("</div>").next())
            .is_some_and(|list| list.contains("href=\"#two\""))
    };
    assert!(!toc(&build()));
    // A settings file appearing is a change of settings.
    let settings = site.join(".config/lsdocs/toc/settings.toml");
    write(&settings, "inline = true\n");
    assert!(toc(&build()));
    // And so is editing it.
    write(&settings, "inline = true\ndepth = 2\n");
    assert!(!toc(&build()));
    write(&settings, "inline = true\ndepth = 3\n");
    assert!(toc(&build()));
    // An unknown setting fails the build, naming the file.
    write(&settings, "detph = 2\n");
    let output = lsdocs(&["build", "--config", config_arg.as_str()]);
    let log = stderr(&output);
    let _ = std::fs::remove_dir_all(&site);
    assert!(!output.status.success());
    assert!(
        log.contains("settings.toml") && log.contains("`detph`"),
        "{log}"
    );
}

#[test]
fn included_files_invalidate_cached_results() {
    let site = temp_dir("deps");
    let docs = site.join("docs");
    write(
        &docs.join("page.rst"),
        "Page\n====\n\n.. include:: part.txt\n\n.. literalinclude:: code.py\n",
    );
    write(&docs.join("part.txt"), "Included *one*.\n");
    write(&docs.join("code.py"), "x = 1\n");
    let config_path = site.join("lsdocs.toml");
    write(
        &config_path,
        "[[plugins]]\nname = \"rst\"\nbuiltin = \"parser-rst\"\n\n\
         [[plugins]]\nname = \"sphinx\"\nbuiltin = \"sphinx-addons\"\n\n\
         [[plugins]]\nname = \"html\"\nbuiltin = \"render-html\"\n",
    );
    let config_arg = config_path.to_string_lossy().into_owned();
    let build = || {
        let output = lsdocs(&["build", "--config", config_arg.as_str()]);
        assert!(output.status.success(), "{}", stderr(&output));
        let page = std::fs::read_to_string(site.join("site/page.html")).unwrap();
        (page, stderr(&output))
    };
    let (page, _) = build();
    assert!(
        page.contains("<em>one</em>") && page.contains("x = 1"),
        "{page}"
    );
    // Unchanged: from the cache.
    let (_, log) = build();
    assert!(log.contains("from the cache"), "{log}");
    // A changed included file is read again, though the page did not change.
    write(&docs.join("part.txt"), "Included *two*.\n");
    write(&docs.join("code.py"), "x = 2\n");
    let (page, _) = build();
    assert!(
        page.contains("<em>two</em>") && page.contains("x = 2"),
        "{page}"
    );
    // A missing file that appears counts as a change too.
    write(
        &docs.join("page.rst"),
        "Page\n====\n\n.. include:: later.txt\n",
    );
    let output = lsdocs(&["build", "--config", config_arg.as_str()]);
    assert!(stderr(&output).contains("later.txt"), "{}", stderr(&output));
    write(&docs.join("later.txt"), "Now *here*.\n");
    let (page, _) = build();
    assert!(page.contains("<em>here</em>"), "{page}");
    let _ = std::fs::remove_dir_all(&site);
}

/// A site documenting a Python package, its plugins in-process or out of
/// process: the API plugin asks the host to parse docstrings
/// (`host/parse`), which must work across a connection too.
fn api_site(transport: Option<&str>) -> BTreeMap<String, Vec<u8>> {
    let site = temp_dir(&format!("api-{}", transport.unwrap_or("builtin")));
    write(
        &site.join("src/pkg/__init__.py"),
        "\"\"\"The *package*.\"\"\"\n\ndef run(x: int = 1) -> str:\n    \"\"\"Run it, see :py:func:`run`.\n\n    Args:\n        x: How often.\n    \"\"\"\n",
    );
    write(
        &site.join("docs/index.md"),
        "# Home\n\nCall {py:func}`pkg.run`.\n",
    );
    write(
        &site.join(".config/lsdocs/python/settings.toml"),
        "paths = [\"src/pkg\"]\n",
    );
    let mut config = String::new();
    for (name, builtin) in [
        ("markdown", "parser-markdown"),
        ("rst", "parser-rst"),
        ("python", "apidoc-python"),
        ("links", "transform-links"),
        ("html", "render-html"),
    ] {
        config.push_str(&format!("\n[[plugins]]\nname = \"{name}\"\n"));
        match transport {
            None => config.push_str(&format!("builtin = \"{builtin}\"\n")),
            Some(t) => config.push_str(&format!(
                "command = ['{LSDOCS}', 'plugin', '{builtin}']\ntransport = \"{t}\"\n"
            )),
        }
    }
    let config_path = site.join("lsdocs.toml");
    write(&config_path, &config);
    let config_arg = config_path.to_string_lossy().into_owned();
    let output = lsdocs(&["build", "--config", config_arg.as_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("warning"), "{}", stderr(&output));
    let files = read_tree(&site.join("site"));
    let _ = std::fs::remove_dir_all(&site);
    files
}

#[test]
fn api_pages_build_the_same_out_of_process() {
    let in_process = api_site(None);
    let files: Vec<_> = in_process
        .keys()
        .map(String::as_str)
        .filter(|path| !path.starts_with("_lsdocs/"))
        .collect();
    assert_eq!(
        files,
        ["api/py/index.html", "api/py/pkg.html", "index.html"]
    );
    let page = String::from_utf8(in_process["api/py/pkg.html"].clone()).unwrap();
    assert!(page.contains("<em>package</em>"), "{page}");
    assert!(page.contains("How often."), "{page}");
    let index = String::from_utf8(in_process["index.html"].clone()).unwrap();
    assert!(
        index.contains("href=\"api/py/pkg.html#py.pkg.run\""),
        "{index}"
    );
    assert_eq!(in_process, api_site(Some("stdio")), "stdio differs");
    assert_eq!(in_process, api_site(Some("pipe")), "pipe differs");
}
