//! Builds `examples/sample-site` and snapshots every output file.
//!
//! After an intended change to the output, update the snapshots with
//! `nur snapshots` and review the diff.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sample_site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample-site")
}

fn files(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type().unwrap().is_dir() {
            files(&entry.path(), &rel, out);
        } else {
            out.push((rel, entry.path()));
        }
    }
}

#[test]
fn sample_site_output() {
    let out = std::env::temp_dir().join(format!("lsdocs-sample-site-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let config = sample_site().join("lsdocs.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_lsdocs"))
        .args(["build", "--config"])
        .arg(&config)
        // An absolute output dir replaces the config's relative one.
        .arg("--output-dir")
        .arg(&out)
        // Leave no cache in the source tree.
        .arg("--no-cache")
        .env_remove("LSDOCS_LOG")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        !stderr.contains("warning") && !stderr.contains("error"),
        "the sample site should build cleanly:\n{stderr}"
    );

    let mut built = Vec::new();
    files(&out, "", &mut built);
    let names: Vec<_> = built.iter().map(|(rel, _)| rel.as_str()).collect();
    assert_eq!(
        names,
        [
            "guide/configuration.html",
            "guide/getting-started.html",
            "img/logo.svg",
            "index.html",
            "reference/cli.html",
            "reference/rst.html",
            "sitemap.xml",
        ]
    );

    for (rel, path) in &built {
        let source = sample_site().join("docs").join(rel);
        if rel.ends_with(".html") || rel == "sitemap.xml" {
            let text = std::fs::read_to_string(path).unwrap();
            insta::assert_snapshot!(rel.replace('/', "__"), text);
        } else {
            assert_eq!(
                std::fs::read(path).unwrap(),
                std::fs::read(&source).unwrap(),
                "asset {rel} should be copied unchanged"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&out);
}
