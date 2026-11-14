//! Plugin registry, supervision and the build pipeline.
//!
//! [`build`] runs one site build:
//!
//! 1. **Discover** files under the source directory.
//! 2. **Start** every configured plugin: builtins in-process, others as child
//!    processes over stdio or a pipe. Each one is initialized, with the list
//!    of discovered files in [`SiteInfo::files`], and its capabilities
//!    recorded.
//! 3. **Parse** each file with the first parser (in config order) that
//!    handles its extension and whose `files` globs match. Files no parser
//!    takes are copied to the output as static assets.
//! 4. **Normalize** the parsed documents (see the `normalize` module):
//!    generated ids for sections and headings, and link targets.
//! 5. **Transform**, one transformer at a time in config order: the collect
//!    pass over every document (if the transformer collects), then the apply
//!    pass. A transformer only sees documents containing a node kind it
//!    selects.
//! 6. **Render** each document with the first renderer, in its first format.
//! 7. **Write** rendered files and assets to the output directory.
//! 8. **Stop** every plugin.
//!
//! Problems with the site's content are returned as diagnostics in the
//! [`BuildReport`]; problems that stop the build are a [`HostError`].
//!
//! Within each phase, documents are processed in parallel on the runtime's
//! worker threads, each plugin handling at most its `max_concurrency`
//! requests at once (by default, one per CPU). Results are gathered in
//! source order, so the output and the order of diagnostics do not depend
//! on timing.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lsdocs_config::Config;
use lsdocs_protocol::doctree::{Document, Value};
use lsdocs_protocol::{
    Diagnostic, InitializeParams, PROTOCOL_VERSION, ParseParams, ProgramInfo, RenderParams,
    RenderSiteParams, RenderedPage, Severity, SiteInfo, TransformCollectParams,
};

mod normalize;
mod plugin;

pub use normalize::slug;

pub use plugin::Builtins;
use plugin::{Events, PluginClient};

/// A failure that stops the build.
#[derive(Debug)]
pub enum HostError {
    /// The configuration cannot be used as written.
    Config(String),
    /// Reading or writing a file failed.
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A plugin names a builtin that does not exist.
    UnknownBuiltin {
        /// Plugin name from the config.
        plugin: String,
        /// The builtin it asked for.
        builtin: String,
        /// Builtins that do exist.
        known: String,
    },
    /// A plugin process could not be started or waited on.
    Launch {
        /// Plugin name from the config.
        plugin: String,
        /// What went wrong.
        error: std::io::Error,
    },
    /// A plugin started but did not complete the handshake.
    Handshake {
        /// Plugin name from the config.
        plugin: String,
        /// What went wrong.
        message: String,
    },
    /// A plugin speaks a different protocol version.
    ProtocolMismatch {
        /// Plugin name from the config.
        plugin: String,
        /// The plugin's version.
        theirs: u32,
        /// The host's version.
        ours: u32,
    },
    /// A plugin request failed.
    Plugin {
        /// Plugin name from the config.
        plugin: String,
        /// The phase, e.g. `"parse"`.
        phase: String,
        /// The source file being processed.
        source_file: String,
        /// What went wrong.
        message: String,
    },
    /// There are documents to render but no plugin can render.
    NoRenderer,
}

impl HostError {
    fn io(path: impl Into<PathBuf>, error: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            error,
        }
    }

    pub(crate) fn launch(plugin: &str, error: std::io::Error) -> Self {
        Self::Launch {
            plugin: plugin.into(),
            error,
        }
    }

    pub(crate) fn handshake(plugin: &str, message: impl Into<String>) -> Self {
        Self::Handshake {
            plugin: plugin.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Config(msg) => write!(f, "config: {msg}"),
            HostError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            HostError::UnknownBuiltin {
                plugin,
                builtin,
                known,
            } => write!(
                f,
                "plugin `{plugin}`: no builtin named `{builtin}` (available: {known})"
            ),
            HostError::Launch { plugin, error } => {
                write!(f, "plugin `{plugin}`: failed to launch: {error}")
            }
            HostError::Handshake { plugin, message } => write!(f, "plugin `{plugin}`: {message}"),
            HostError::ProtocolMismatch {
                plugin,
                theirs,
                ours,
            } => write!(
                f,
                "plugin `{plugin}` speaks protocol version {theirs}, but this lsdocs speaks {ours}"
            ),
            HostError::Plugin {
                plugin,
                phase,
                source_file,
                message,
            } => write!(
                f,
                "plugin `{plugin}` failed to {phase} {source_file}: {message}"
            ),
            HostError::NoRenderer => f.write_str("no configured plugin can render documents"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HostError::Io { error, .. } | HostError::Launch { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// What a build did.
#[derive(Debug, Default)]
pub struct BuildReport {
    /// Documents parsed and rendered.
    pub documents: usize,
    /// Files copied unchanged.
    pub assets: usize,
    /// Files written, relative to the output directory, with `/` separators.
    pub written: Vec<String>,
    /// Problems reported by plugins, in the order they arrived.
    pub diagnostics: Vec<Diagnostic>,
    /// Source text of each parsed document, for locating diagnostics.
    sources: HashMap<String, String>,
}

impl BuildReport {
    /// Whether any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Formats a diagnostic as `severity[code]: file:line:col: message`.
    pub fn format_diagnostic(&self, d: &Diagnostic) -> String {
        let severity = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        };
        let code = d
            .code
            .as_deref()
            .map(|c| format!("[{c}]"))
            .unwrap_or_default();
        let location = match (&d.source, d.span) {
            (Some(source), Some(span)) => match self.sources.get(source) {
                Some(text) => {
                    let (line, col) = line_col(text, span.start as usize);
                    format!("{source}:{line}:{col}: ")
                }
                None => format!("{source}: "),
            },
            (Some(source), None) => format!("{source}: "),
            _ => String::new(),
        };
        format!("{severity}{code}: {location}{}", d.message)
    }
}

/// What parsing one file produced.
enum Parsed {
    /// The file is not UTF-8 text.
    NotUtf8(String),
    /// Source path, source text and the parser's result.
    Document(String, String, lsdocs_protocol::ParseResult),
}

/// Runs `f` on every item, several at once on the runtime's worker threads,
/// and returns the results in the order of `items`. Each plugin limits its
/// own concurrency; this only bounds how many tasks are queued at a time.
async fn in_parallel<T, R, F, Fut>(items: Vec<T>, f: F) -> Vec<R>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut,
    Fut: Future<Output = R> + Send + 'static,
{
    use futures_util::stream::{self, StreamExt};
    let width = plugin::default_concurrency() * 2;
    stream::iter(items)
        .map(|item| {
            let task = tokio::spawn(f(item));
            async move {
                match task.await {
                    Ok(result) => result,
                    Err(e) => std::panic::resume_unwind(e.into_panic()),
                }
            }
        })
        .buffered(width)
        .collect()
        .await
}

/// One-based line and column (in characters) of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, col)
}

/// Builds the site configured by `config`, whose paths are relative to
/// `root` (the directory holding `lsdocs.toml`).
pub async fn build(
    config: &Config,
    root: &Path,
    builtins: &Builtins,
) -> Result<BuildReport, HostError> {
    let events = Events::default();
    let files = discover(&root.join(&config.source_dir))?;
    let init = InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        host: ProgramInfo {
            name: "lsdocs".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        site: SiteInfo {
            title: config.title.clone(),
            base_url: config.base_url.clone(),
            root: std::path::absolute(root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            files,
        },
        options: Value::Null,
    };

    let mut plugins = Vec::with_capacity(config.plugins.len());
    let mut result = Ok(());
    for entry in &config.plugins {
        match PluginClient::start(entry, &init, root, builtins, &events).await {
            Ok(plugin) => plugins.push(Arc::new(plugin)),
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }

    let report = match result {
        Ok(()) => run_pipeline(config, root, &init.site.files, &plugins).await,
        Err(e) => Err(e),
    };

    // Always stop whatever was started. Every task using a plugin has
    // finished, so each `Arc` is the last one.
    for plugin in plugins {
        let Ok(plugin) = Arc::try_unwrap(plugin) else {
            tracing::warn!("a plugin is still in use at shutdown");
            continue;
        };
        if let Err(e) = plugin.shutdown().await {
            tracing::warn!("{e}");
        }
    }

    let mut report = report?;
    // Out-of-band diagnostics arrive as notifications; collect them last so
    // none sent before shutdown are missed.
    report.diagnostics.extend(events.take_diagnostics());
    Ok(report)
}

async fn run_pipeline(
    config: &Config,
    root: &Path,
    files: &[String],
    plugins: &[Arc<PluginClient>],
) -> Result<BuildReport, HostError> {
    let mut report = BuildReport::default();
    let source_dir = root.join(&config.source_dir);
    let output_dir = root.join(&config.output_dir);

    // Discover and parse. Files no parser takes are assets.
    let mut to_parse = Vec::new();
    let mut assets = Vec::new();
    for source in files.iter().cloned() {
        let ext = source.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let parser = plugins.iter().find(|p| {
            p.capabilities
                .parse
                .as_ref()
                .is_some_and(|cap| cap.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
                && p.wants_file(&source)
        });
        match parser {
            Some(parser) => to_parse.push((source, Arc::clone(parser))),
            None => assets.push(source),
        }
    }
    let parsed = in_parallel(to_parse, {
        let source_dir = source_dir.clone();
        move |(source, parser)| {
            let path = source_dir.join(&source);
            async move {
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                        return Ok(Parsed::NotUtf8(source));
                    }
                    Err(e) => return Err(HostError::io(path, e)),
                };
                let result = parser
                    .parse(ParseParams {
                        source: source.clone(),
                        text: text.clone(),
                    })
                    .await?;
                Ok(Parsed::Document(source, text, result))
            }
        }
    })
    .await;
    let mut documents = Vec::new();
    for parsed in parsed {
        match parsed? {
            Parsed::NotUtf8(source) => report.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                message: "file is not valid UTF-8".into(),
                source: Some(source),
                ..Diagnostic::default()
            }),
            Parsed::Document(source, text, result) => {
                add_diagnostics(&mut report, &source, result.diagnostics);
                let mut document = result.document;
                document.source = source.clone();
                report.sources.insert(source, text);
                documents.push(document);
            }
        }
    }

    // Normalize: generated ids and link targets, the same for every parser.
    let problems = normalize::normalize(&mut documents);
    report.diagnostics.extend(problems);

    // Transform, one transformer at a time; within a pass, documents in
    // parallel.
    for plugin in plugins
        .iter()
        .filter(|p| p.capabilities.transform.is_some())
    {
        let collects = plugin
            .capabilities
            .transform
            .as_ref()
            .is_some_and(|t| t.collect);
        let selected: Vec<usize> = (0..documents.len())
            .filter(|&i| plugin.wants_document(&documents[i]))
            .collect();
        if collects {
            let inputs = selected.iter().map(|&i| documents[i].clone()).collect();
            let results = in_parallel(inputs, {
                let plugin = Arc::clone(plugin);
                move |document: Document| {
                    let plugin = Arc::clone(&plugin);
                    async move {
                        let source = document.source.clone();
                        let result = plugin.collect(TransformCollectParams { document }).await;
                        result.map(|r| (source, r))
                    }
                }
            })
            .await;
            let mut data = serde_json::Map::new();
            for result in results {
                let (source, result) = result?;
                add_diagnostics(&mut report, &source, result.diagnostics);
                data.insert(source, result.data);
            }
            plugin.set_collected(Value::Object(data)).await?;
        }
        let inputs: Vec<(usize, Document)> = selected
            .iter()
            .map(|&i| {
                let placeholder = Document::new(documents[i].source.clone());
                (i, std::mem::replace(&mut documents[i], placeholder))
            })
            .collect();
        let results = in_parallel(inputs, {
            let plugin = Arc::clone(plugin);
            move |(i, document): (usize, Document)| {
                let plugin = Arc::clone(&plugin);
                async move { plugin.apply(document).await.map(|r| (i, r)) }
            }
        })
        .await;
        for result in results {
            let (i, result) = result?;
            let source = std::mem::take(&mut documents[i].source);
            add_diagnostics(&mut report, &source, result.diagnostics);
            documents[i] = result.document;
            documents[i].source = source;
        }
    }

    // Render, documents in parallel.
    let mut outputs: Vec<(String, Vec<u8>)> = Vec::new();
    if !documents.is_empty() {
        let renderer = plugins
            .iter()
            .find(|p| {
                p.capabilities
                    .render
                    .as_ref()
                    .is_some_and(|r| !r.formats.is_empty())
            })
            .ok_or(HostError::NoRenderer)?;
        let format = renderer
            .capabilities
            .render
            .as_ref()
            .expect("checked above")
            .formats[0]
            .clone();
        let site = renderer
            .capabilities
            .render
            .as_ref()
            .is_some_and(|r| r.site);
        let results = in_parallel(documents, {
            let renderer = Arc::clone(renderer);
            let format = format.clone();
            move |document: Document| {
                let renderer = Arc::clone(&renderer);
                let format = format.clone();
                async move {
                    let source = document.source.clone();
                    let metadata = if site {
                        document.metadata.clone()
                    } else {
                        Default::default()
                    };
                    let result = renderer.render(RenderParams { document, format }).await;
                    result.map(|r| (source, metadata, r))
                }
            }
        })
        .await;
        let mut pages = Vec::new();
        for result in results {
            let (source, metadata, result) = result?;
            add_diagnostics(&mut report, &source, result.diagnostics);
            let stem = match source.rsplit_once('.') {
                Some((stem, _)) if !stem.is_empty() && !stem.ends_with('/') => stem,
                _ => &source,
            };
            let path = format!("{stem}.{}", result.extension);
            outputs.push((path.clone(), result.output));
            pages.push(RenderedPage {
                source,
                path,
                metadata,
            });
            report.documents += 1;
        }
        if site {
            let result = renderer
                .render_site(RenderSiteParams {
                    format: format.clone(),
                    pages,
                })
                .await?;
            report.diagnostics.extend(result.diagnostics);
            for file in result.files {
                let taken =
                    outputs.iter().any(|(p, _)| *p == file.path) || assets.contains(&file.path);
                if !is_safe_output_path(&file.path) || taken {
                    report.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        message: format!(
                            "plugin `{}` returned a site file with an invalid or taken path `{}`",
                            renderer.name, file.path
                        ),
                        ..Diagnostic::default()
                    });
                    continue;
                }
                outputs.push((file.path, file.output));
            }
        }
    }

    // Write.
    for (rel, bytes) in &outputs {
        write_file(&output_dir.join(rel), bytes)?;
        report.written.push(rel.clone());
    }
    for rel in assets {
        let from = source_dir.join(&rel);
        let to = output_dir.join(&rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        std::fs::copy(&from, &to).map_err(|e| HostError::io(&from, e))?;
        report.written.push(rel);
        report.assets += 1;
    }
    Ok(report)
}

fn add_diagnostics(report: &mut BuildReport, source: &str, diagnostics: Vec<Diagnostic>) {
    report
        .diagnostics
        .extend(diagnostics.into_iter().map(|mut d| {
            d.source.get_or_insert_with(|| source.to_owned());
            d
        }));
}

/// Whether a plugin-chosen output path stays inside the output directory:
/// relative, `/`-separated, with no empty, `.` or `..` segments.
fn is_safe_output_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), HostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| HostError::io(path, e))
}

/// Files under `dir`, relative to it with `/` separators, sorted. Hidden
/// files and directories (starting with `.`) are skipped.
fn discover(dir: &Path) -> Result<Vec<String>, HostError> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), HostError> {
        let entries = std::fs::read_dir(dir).map_err(|e| HostError::io(dir, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| HostError::io(dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let file_type = entry
                .file_type()
                .map_err(|e| HostError::io(entry.path(), e))?;
            if file_type.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn in_parallel_keeps_input_order() {
        // Later items finish first; results still come back in input order.
        let items: Vec<u64> = (0..20).collect();
        let results = in_parallel(items, |i| async move {
            tokio::time::sleep(std::time::Duration::from_millis(20 - i)).await;
            i * 10
        })
        .await;
        assert_eq!(results, (0..20).map(|i| i * 10).collect::<Vec<_>>());
    }

    #[test]
    fn output_paths_stay_inside_the_output_dir() {
        for ok in ["sitemap.xml", "a/b.json"] {
            assert!(is_safe_output_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/etc/x",
            "../x",
            "a/../../x",
            "a//b",
            "./x",
            "C:/x",
            "a\\b",
        ] {
            assert!(!is_safe_output_path(bad), "{bad}");
        }
    }

    #[test]
    fn line_col_counts_lines_and_characters() {
        let text = "ab\ncdé\nf";
        assert_eq!(line_col(text, 0), (1, 1));
        assert_eq!(line_col(text, 3), (2, 1));
        assert_eq!(line_col(text, 7), (2, 4)); // after "cdé" (é is 2 bytes)
        assert_eq!(line_col(text, 6), (2, 3)); // inside é: clamps to its start
        assert_eq!(line_col(text, 99), (3, 2));
    }

    #[test]
    fn diagnostics_are_formatted_with_locations() {
        let mut report = BuildReport::default();
        report.sources.insert("a.md".into(), "one\ntwo".into());
        let d = Diagnostic {
            severity: Severity::Warning,
            message: "bad link".into(),
            source: Some("a.md".into()),
            span: Some(lsdocs_protocol::doctree::Span::new(5, 6)),
            code: Some("broken-link".into()),
        };
        assert_eq!(
            report.format_diagnostic(&d),
            "warning[broken-link]: a.md:2:2: bad link"
        );
        let bare = Diagnostic {
            message: "plugin crashed".into(),
            ..Diagnostic::default()
        };
        assert_eq!(report.format_diagnostic(&bare), "error: plugin crashed");
    }
}
