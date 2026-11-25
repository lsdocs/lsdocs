//! A plugin written from `spec/wire.md` alone, with no lsdocs SDK:
//! `examples/plugins/txt-parser/txt_parser.py`, standard-library Python.
//! Building a site with it checks that the spec is enough to implement the
//! protocol. Skipped when no Python is installed.

// These tests build sites with the built-in plugins.
#![cfg(all(
    feature = "extras",
    feature = "parser-myst",
    feature = "parser-rst",
    feature = "render-html",
    feature = "sphinx-addons",
    feature = "transform-highlight",
    feature = "transform-links",
    feature = "transform-nav",
    feature = "transform-toc"
))]
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

const LSDOCS: &str = env!("CARGO_BIN_EXE_lsdocs");

/// A Python interpreter: `LSDOCS_PYTHON`, else `python3` or `python`.
fn python() -> Option<String> {
    let candidates = std::env::var("LSDOCS_PYTHON")
        .into_iter()
        .chain(["python3".to_owned(), "python".to_owned()]);
    candidates.into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn a_plugin_written_from_the_spec_builds_a_site() {
    let Some(python) = python() else {
        eprintln!("skipped: no Python found");
        return;
    };
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/plugins/txt-parser/txt_parser.py")
        .canonicalize()
        .unwrap();
    let site = std::env::temp_dir().join(format!("lsdocs-test-{}-no-sdk", std::process::id()));
    let _ = std::fs::remove_dir_all(&site);
    std::fs::create_dir_all(site.join("docs")).unwrap();
    let command = toml_string(&python);
    let script_arg = toml_string(&script.to_string_lossy());
    std::fs::write(
        site.join("lsdocs.toml"),
        format!(
            "[[plugins]]\nname = \"txt\"\ncommand = [{command}, {script_arg}]\n\n\
             [[plugins]]\nname = \"html\"\nbuiltin = \"render-html\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        site.join("docs/index.txt"),
        "Hello from a plugin\nwith no SDK.\n\nSecond paragraph.\n",
    )
    .unwrap();
    let output = Command::new(LSDOCS)
        .args(["build", "--no-cache", "--config"])
        .arg(site.join("lsdocs.toml"))
        .env_remove("LSDOCS_LOG")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let html = std::fs::read_to_string(site.join("site/index.html")).unwrap();
    let _ = std::fs::remove_dir_all(&site);
    assert!(
        html.contains("<p>Hello from a plugin\nwith no SDK.</p>"),
        "{html}"
    );
    assert!(html.contains("<p>Second paragraph.</p>"), "{html}");
}

/// `s` as a TOML basic string.
fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
