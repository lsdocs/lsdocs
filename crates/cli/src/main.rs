//! The lsdocs command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lsdocs_config::{Config, Overrides};
use lsdocs_host::{BuildOptions, Builtins};
mod keys;
mod log;
mod serve;

/// Build documentation sites with language-agnostic plugins.
#[derive(Debug, Parser)]
#[command(name = "lsdocs", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Show more detail (`-v` debug, `-vv` trace). `LSDOCS_LOG` overrides
    /// `-v` and `-q`.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Show only warnings and errors, not progress.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

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
        /// Don't reuse or store results in the build cache
        /// (`.lsdocs-cache/` next to the config).
        #[arg(long)]
        no_cache: bool,
    },
    /// Build the site, serve it, and rebuild and reload pages on changes.
    Serve {
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
        /// Don't reuse or store results in the build cache.
        #[arg(long)]
        no_cache: bool,
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1")]
        host: std::net::IpAddr,
        /// Port to listen on (0 picks a free one).
        #[arg(short, long, default_value_t = 8000)]
        port: u16,
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
        /// Builtin name, e.g. `parser-myst`.
        name: String,
    },
}

/// The first-party plugins, available as `builtin = "<name>"`.
fn builtins() -> Builtins {
    let builtins = Builtins::new();
    #[cfg(feature = "apidoc-clang")]
    let builtins = builtins
        .with("apidoc-c", lsdocs_apidoc_clang::plugin_c)
        .with("apidoc-cpp", lsdocs_apidoc_clang::plugin_cpp);
    #[cfg(feature = "apidoc-python")]
    let builtins = builtins.with("apidoc-python", lsdocs_apidoc_python::plugin);
    #[cfg(feature = "apidoc-rust")]
    let builtins = builtins.with("apidoc-rust", lsdocs_apidoc_rust::plugin);
    #[cfg(feature = "apidoc-typescript")]
    let builtins = builtins.with("apidoc-typescript", lsdocs_apidoc_typescript::plugin);
    #[cfg(feature = "parser-myst")]
    let builtins = builtins.with("parser-myst", lsdocs_parser_markdown::myst::plugin);
    #[cfg(feature = "parser-pulldown-cmark")]
    let builtins = builtins.with(
        "parser-pulldown-cmark",
        lsdocs_parser_markdown::pulldown_cmark::plugin,
    );
    #[cfg(feature = "parser-rst")]
    let builtins = builtins.with("parser-rst", lsdocs_parser_rst::plugin);
    #[cfg(feature = "sphinx-addons")]
    let builtins = builtins.with("sphinx-addons", lsdocs_sphinx_addons::plugin);
    #[cfg(feature = "extras")]
    let builtins = builtins.with("extras", lsdocs_extras_plugin::plugin);
    #[cfg(feature = "transform-highlight")]
    let builtins = builtins.with("transform-highlight", lsdocs_transform_highlight::plugin);
    #[cfg(feature = "transform-links")]
    let builtins = builtins.with("transform-links", lsdocs_transform_links::plugin);
    #[cfg(feature = "transform-nav")]
    let builtins = builtins.with("transform-nav", lsdocs_transform_nav::plugin);
    #[cfg(feature = "transform-toc")]
    let builtins = builtins.with("transform-toc", lsdocs_transform_toc::plugin);
    #[cfg(feature = "render-html")]
    let builtins = builtins.with("render-html", lsdocs_render_html::plugin);
    builtins
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    log::init(log::Verbosity {
        verbose: cli.verbose,
        quiet: cli.quiet,
        trace_rpc: cli.trace_rpc,
        // A plugin process's stderr is the host's log: only problems.
        default: match cli.command {
            Commands::Plugin { .. } => "warn",
            _ => "info",
        },
    });
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
            no_cache,
        } => runtime.block_on(build(
            &config,
            Overrides {
                source_dir,
                output_dir,
                base_url,
            },
            &BuildOptions { cache: !no_cache },
        )),
        Commands::Serve {
            config,
            source_dir,
            output_dir,
            base_url,
            no_cache,
            host,
            port,
        } => runtime
            .block_on(serve::serve(
                &config,
                &Overrides {
                    source_dir,
                    output_dir,
                    base_url,
                },
                &builtins(),
                &serve::ServeOptions {
                    addr: (host, port).into(),
                    build: BuildOptions { cache: !no_cache },
                },
            ))
            .map(|()| ExitCode::SUCCESS),
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

/// Reads the config file, then applies `LSDOCS_*` variables and `cli`.
fn load_config(config_path: &Path, cli: Overrides) -> Result<Config, String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let mut config =
        Config::from_toml(&text).map_err(|e| format!("{}: {e}", config_path.display()))?;
    config.apply(Overrides::from_env(std::env::vars()));
    config.apply(cli);
    Ok(config)
}

/// The site's root: the directory holding the config file.
fn site_root(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

async fn build(
    config_path: &Path,
    cli: Overrides,
    options: &BuildOptions,
) -> Result<ExitCode, String> {
    let config = load_config(config_path, cli)?;
    let root = site_root(config_path);

    let report = lsdocs_host::build(&config, &root, &builtins(), options)
        .await
        .map_err(|e| e.to_string())?;
    for d in &report.diagnostics {
        eprintln!("{}", report.format_diagnostic(d));
    }
    let cached = match report.cached {
        0 => String::new(),
        n => format!(" ({n} result(s) from the cache)"),
    };
    eprintln!(
        "built {} document(s) and copied {} asset(s) into {}{cached}",
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
builtin = "parser-myst"

[[plugins]]
name = "rst"
builtin = "parser-rst"

[[plugins]]
name = "sphinx"
builtin = "sphinx-addons"

[[plugins]]
name = "extras"
builtin = "extras"

[[plugins]]
name = "toc"
builtin = "transform-toc"

[[plugins]]
name = "links"
builtin = "transform-links"

[[plugins]]
name = "nav"
builtin = "transform-nav"

[[plugins]]
name = "highlight"
builtin = "transform-highlight"

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
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        write(&gitignore, "/site/\n/.lsdocs-cache/\n")?;
    }
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

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(all(
        feature = "apidoc-clang",
        feature = "apidoc-python",
        feature = "apidoc-rust",
        feature = "apidoc-typescript",
        feature = "extras",
        feature = "parser-myst",
        feature = "parser-pulldown-cmark",
        feature = "parser-rst",
        feature = "render-html",
        feature = "sphinx-addons",
        feature = "transform-highlight",
        feature = "transform-links",
        feature = "transform-nav",
        feature = "transform-toc",
    ))]
    fn builtins_follow_features() {
        assert_eq!(
            super::builtins().names(),
            [
                "apidoc-c",
                "apidoc-cpp",
                "apidoc-python",
                "apidoc-rust",
                "apidoc-typescript",
                "extras",
                "parser-myst",
                "parser-pulldown-cmark",
                "parser-rst",
                "render-html",
                "sphinx-addons",
                "transform-highlight",
                "transform-links",
                "transform-nav",
                "transform-toc",
            ]
        );
    }

    #[test]
    fn no_features_no_builtins() {
        #[cfg(not(any(
            feature = "apidoc-clang",
            feature = "apidoc-python",
            feature = "apidoc-rust",
            feature = "apidoc-typescript",
            feature = "extras",
            feature = "parser-myst",
            feature = "parser-pulldown-cmark",
            feature = "parser-rst",
            feature = "render-html",
            feature = "sphinx-addons",
            feature = "transform-highlight",
            feature = "transform-links",
            feature = "transform-nav",
            feature = "transform-toc",
        )))]
        assert!(super::builtins().names().is_empty());
    }
}
