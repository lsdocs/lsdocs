//! The documentation site (`docs/`, built with lsdocs) builds without
//! warnings, so broken links and malformed pages fail the tests.

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
    feature = "transform-toc",
    feature = "apidoc-clang",
    feature = "apidoc-python",
    feature = "apidoc-rust",
    feature = "apidoc-typescript"
))]
#![allow(clippy::unwrap_used)]

use std::process::Command;

const LSDOCS: &str = env!("CARGO_BIN_EXE_lsdocs");

#[test]
fn docs_site_builds_cleanly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::env::temp_dir().join(format!("lsdocs-test-{}-docs", std::process::id()));
    let output = Command::new(LSDOCS)
        .args(["build", "--no-cache", "--config"])
        .arg(root.join("docs/lsdocs.toml"))
        .arg("--output-dir")
        .arg(&out)
        .env_remove("LSDOCS_LOG")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&out);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        !stderr.contains("warning") && !stderr.contains("error"),
        "the docs site should build cleanly:\n{stderr}"
    );
}
