//! The lsdocs command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lsdocs_config::{Config, Overrides};
use lsdocs_host::Builtins;
use tracing_subscriber::EnvFilter;

/// Build documentation sites with language-agnostic plugins.
#[derive(Debug, Parser)]
#[command(name = "lsdocs", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Show more detail (`-v` info, `-vv` debug). `LSDOCS_LOG` overrides.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Log every protocol message exchanged with plugins, as JSON.
    #[arg(long, global = true)]
    trace_rpc: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Build the site.
    Build {
        /// Path to the config file. Paths in it are relative to its directory.
        #[arg(short, long, default_value = "lsdocs.toml")]
        config: PathBuf,
        /// Override `build.source_dir`.
        #[arg(long)]
        source_dir: Option<String>,
        /// Override `build.output_dir`.
        #[arg(long)]
        output_dir: Option<String>,
        /// Override `site.base_url`.
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Create a starter site: `lsdocs.toml` and `docs/index.md`.
    Init {
        /// Directory to create the site in.
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Serve a builtin plugin over stdio (or `LSDOCS_PIPE`), as an
    /// out-of-process plugin. Used for testing.
    #[command(hide = true)]
    Plugin {
        /// Builtin name, e.g. `parser-markdown`.
        name: String,
    },
}

/// The first-party plugins, available as `builtin = "<name>"`.
fn builtins() -> Builtins {
    Builtins::new()
        .with("parser-markdown", lsdocs_parser_markdown::plugin)
        .with("transform-toc", lsdocs_transform_toc::plugin)
        .with("render-html", lsdocs_render_html::plugin)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.trace_rpc);
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Commands::Build {
            config,
            source_dir,
            output_dir,
            base_url,
        } => runtime.block_on(build(
            &config,
            Overrides {
                source_dir,
                output_dir,
                base_url,
            },
        )),
        Commands::Init { dir } => init(&dir),
        Commands::Plugin { name } => runtime.block_on(serve_builtin(&name)),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(verbose: u8, trace_rpc: bool) {
    let filter = EnvFilter::try_from_env("LSDOCS_LOG").unwrap_or_else(|_| {
        let level = match verbose {
            0 => "warn",
            1 => "info",
            _ => "debug",
        };
        let mut filter = EnvFilter::new(level);
        if trace_rpc {
            filter = filter.add_directive(
                format!("{}=trace", lsdocs_plugin_sdk::TRACE_TARGET)
                    .parse()
                    .expect("valid directive"),
            );
        }
        filter
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

async fn build(config_path: &Path, cli: Overrides) -> Result<ExitCode, String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let mut config =
        Config::from_toml(&text).map_err(|e| format!("{}: {e}", config_path.display()))?;
    config.apply(Overrides::from_env(std::env::vars()));
    config.apply(cli);
    let root = match config_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    let report = lsdocs_host::build(&config, &root, &builtins())
        .await
        .map_err(|e| e.to_string())?;
    for d in &report.diagnostics {
        eprintln!("{}", report.format_diagnostic(d));
    }
    eprintln!(
        "built {} document(s) and copied {} asset(s) into {}",
        report.documents,
        report.assets,
        root.join(&config.output_dir).display()
    );
    Ok(if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

const STARTER_CONFIG: &str = r#"[site]
title = "My Docs"

[[plugins]]
name = "markdown"
builtin = "parser-markdown"

[[plugins]]
name = "toc"
builtin = "transform-toc"

[[plugins]]
name = "html"
builtin = "render-html"
"#;

const STARTER_INDEX: &str = "# Welcome\n\nThis site is built with lsdocs.\n\n## Writing pages\n\nAdd Markdown files under `docs/`.\n\n## Building\n\nRun `lsdocs build` to write the site to `site/`.\n";

fn init(dir: &Path) -> Result<ExitCode, String> {
    let config = dir.join("lsdocs.toml");
    let index = dir.join("docs").join("index.md");
    for path in [&config, &index] {
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
    }
    let write = |path: &Path, text: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write(&config, STARTER_CONFIG)?;
    write(&index, STARTER_INDEX)?;
    eprintln!("created {} and {}", config.display(), index.display());
    Ok(ExitCode::SUCCESS)
}

async fn serve_builtin(name: &str) -> Result<ExitCode, String> {
    let builtins = builtins();
    let plugin = builtins.make(name).ok_or_else(|| {
        format!(
            "no builtin named `{name}` (available: {})",
            builtins.names().join(", ")
        )
    })?;
    lsdocs_plugin_sdk::serve(plugin)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ExitCode::SUCCESS)
}
