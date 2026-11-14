//! Starting, talking to and stopping plugins.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use globset::{Glob, GlobSet, GlobSetBuilder};
use lsdocs_config::{Command, Launch, Transport as TransportKind};
use lsdocs_plugin_sdk::{BoxFuture, Context, Event, EventSink, Plugin};
use lsdocs_protocol::doctree::Document;
use lsdocs_protocol::{
    Capabilities, Diagnostic, InitializeParams, LogLevel, PROTOCOL_VERSION, ParseParams,
    ParseResult, RenderParams, RenderResult, TransformApplyParams, TransformApplyResult,
    TransformCollectParams, TransformCollectResult, notification, request,
};
use lsdocs_rpc::transport::{Transport, pipe};
use lsdocs_rpc::{Options, Peer, Router};
use tokio::process::Child;
use tokio::task::JoinHandle;

use crate::HostError;

/// How long a plugin gets to start and answer `initialize`.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a plugin gets to exit after `exit` before it is killed.
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// First-party plugins the host can run in-process, by name.
#[derive(Clone, Default)]
pub struct Builtins {
    plugins: HashMap<String, fn() -> Plugin>,
}

impl std::fmt::Debug for Builtins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<_> = self.plugins.keys().collect();
        names.sort();
        f.debug_tuple("Builtins").field(&names).finish()
    }
}

impl Builtins {
    /// No builtins.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a builtin under `name`.
    pub fn with(mut self, name: impl Into<String>, make: fn() -> Plugin) -> Self {
        self.plugins.insert(name.into(), make);
        self
    }

    /// Creates the builtin named `name`, if there is one.
    pub fn make(&self, name: &str) -> Option<Plugin> {
        self.plugins.get(name).map(|make| make())
    }

    /// Registered names, sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<_> = self.plugins.keys().map(String::as_str).collect();
        names.sort();
        names
    }
}

/// Collects what plugins report outside of results.
#[derive(Clone, Default)]
pub(crate) struct Events {
    diagnostics: Arc<Mutex<Vec<Diagnostic>>>,
}

impl Events {
    fn handle(&self, plugin: &str, event: Event) {
        match event {
            Event::Log(log) => match log.level {
                LogLevel::Error => tracing::error!(plugin, "{}", log.message),
                LogLevel::Warn => tracing::warn!(plugin, "{}", log.message),
                LogLevel::Info => tracing::info!(plugin, "{}", log.message),
                LogLevel::Debug => tracing::debug!(plugin, "{}", log.message),
                LogLevel::Trace => tracing::trace!(plugin, "{}", log.message),
            },
            Event::Progress(p) => match p.percentage {
                Some(pct) => tracing::debug!(plugin, "{} ({pct}%)", p.message),
                None => tracing::debug!(plugin, "{}", p.message),
            },
            Event::Diagnostics(d) => self.diagnostics.lock().expect("lock").extend(d.diagnostics),
        }
    }

    /// Takes the diagnostics reported so far.
    pub(crate) fn take_diagnostics(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics.lock().expect("lock"))
    }
}

/// Events from an in-process plugin, delivered straight to [`Events`].
struct InProcessSink {
    plugin: String,
    events: Events,
}

impl EventSink for InProcessSink {
    fn send(&self, event: Event) -> BoxFuture<'_, ()> {
        self.events.handle(&self.plugin, event);
        Box::pin(async {})
    }
}

enum Kind {
    InProcess {
        plugin: Box<Plugin>,
        ctx: Context,
    },
    Remote {
        peer: Peer,
        connection: JoinHandle<Result<(), lsdocs_rpc::Error>>,
        child: Box<Child>,
    },
}

/// A started, initialized plugin.
pub(crate) struct PluginClient {
    /// The name from the site config.
    pub name: String,
    /// What it said it can do.
    pub capabilities: Capabilities,
    /// The `files` globs from the site config, if any.
    files: Option<GlobSet>,
    kind: Kind,
}

impl PluginClient {
    /// Starts and initializes the plugin described by `entry`.
    pub async fn start(
        entry: &lsdocs_config::Plugin,
        init: &InitializeParams,
        root: &Path,
        builtins: &Builtins,
        events: &Events,
    ) -> Result<Self, HostError> {
        let name = entry.name.clone();
        let files = compile_globs(&name, &entry.files)?;
        let mut init = init.clone();
        init.options = entry.options.clone();

        let (kind, capabilities) = match &entry.launch {
            Launch::Builtin { name: builtin } => {
                let plugin = builtins
                    .make(builtin)
                    .ok_or_else(|| HostError::UnknownBuiltin {
                        plugin: name.clone(),
                        builtin: builtin.clone(),
                        known: builtins.names().join(", "),
                    })?;
                let sink = Arc::new(InProcessSink {
                    plugin: name.clone(),
                    events: events.clone(),
                });
                let ctx = Context::new(sink, &init);
                let capabilities = plugin.capabilities();
                (
                    Kind::InProcess {
                        plugin: Box::new(plugin),
                        ctx,
                    },
                    capabilities,
                )
            }
            Launch::External { command, transport } => {
                let mut cmd = command_for(command, root);
                cmd.current_dir(root)
                    .kill_on_drop(true)
                    .stderr(Stdio::inherit());
                let (transport, child) = match transport {
                    TransportKind::Stdio => {
                        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
                        let mut child = spawn(&name, &mut cmd)?;
                        let stdout = child.stdout.take().expect("piped");
                        let stdin = child.stdin.take().expect("piped");
                        (Transport::new(stdout, stdin), child)
                    }
                    TransportKind::Pipe => {
                        let pipe_name = pipe::unique_name();
                        let listener = pipe::Listener::bind(&pipe_name)
                            .map_err(|e| HostError::launch(&name, e))?;
                        cmd.env(pipe::ENV_VAR, &pipe_name)
                            .stdin(Stdio::null())
                            .stdout(Stdio::inherit());
                        let mut child = spawn(&name, &mut cmd)?;
                        let transport = tokio::select! {
                            accepted = tokio::time::timeout(STARTUP_TIMEOUT, listener.accept()) => match accepted {
                                Ok(Ok(t)) => t,
                                Ok(Err(e)) => return Err(HostError::launch(&name, e)),
                                Err(_) => return Err(HostError::handshake(&name, "did not connect to the pipe in time")),
                            },
                            status = child.wait() => {
                                let detail = match status {
                                    Ok(s) => format!("exited ({s}) before connecting"),
                                    Err(e) => format!("could not be waited on: {e}"),
                                };
                                return Err(HostError::handshake(&name, detail));
                            }
                        };
                        (transport, child)
                    }
                };

                let router = router(&name, events);
                let (peer, connection) = lsdocs_rpc::connect(transport, router, Options::default());
                let connection = tokio::spawn(connection.run());
                let result = tokio::time::timeout(
                    STARTUP_TIMEOUT,
                    peer.request::<request::Initialize>(&init),
                )
                .await
                .map_err(|_| HostError::handshake(&name, "did not answer `initialize` in time"))?
                .map_err(|e| HostError::handshake(&name, e.to_string()))?;
                if result.protocol_version != PROTOCOL_VERSION {
                    return Err(HostError::ProtocolMismatch {
                        plugin: name,
                        theirs: result.protocol_version,
                        ours: PROTOCOL_VERSION,
                    });
                }
                peer.notify::<notification::Initialized>(&())
                    .await
                    .map_err(|e| HostError::handshake(&name, e.to_string()))?;
                tracing::debug!(plugin = %name, remote = %result.plugin.name, version = %result.plugin.version, "plugin started");
                (
                    Kind::Remote {
                        peer,
                        connection,
                        child: Box::new(child),
                    },
                    result.capabilities,
                )
            }
        };

        Ok(Self {
            name,
            capabilities,
            files,
            kind,
        })
    }

    /// Whether this plugin should see `source` (by its `files` globs).
    pub fn wants_file(&self, source: &str) -> bool {
        self.files.as_ref().is_none_or(|set| set.is_match(source))
    }

    /// Whether a transformer should see `doc`: its file matches and it
    /// contains one of the node kinds the transformer selects.
    pub fn wants_document(&self, doc: &Document) -> bool {
        if !self.wants_file(&doc.source) {
            return false;
        }
        let Some(transform) = &self.capabilities.transform else {
            return false;
        };
        transform.nodes.is_empty()
            || doc
                .descendants()
                .any(|n| transform.nodes.iter().any(|k| *k == n.kind()))
    }

    pub async fn parse(&self, params: ParseParams) -> Result<ParseResult, HostError> {
        let source = params.source.clone();
        match &self.kind {
            Kind::InProcess { plugin, ctx } => {
                plugin.parse(params, ctx).await.map_err(|e| e.to_string())
            }
            Kind::Remote { peer, .. } => peer
                .request::<request::Parse>(&params)
                .await
                .map_err(|e| e.to_string()),
        }
        .map_err(|message| self.phase_error("parse", &source, message))
    }

    pub async fn collect(
        &self,
        params: TransformCollectParams,
    ) -> Result<TransformCollectResult, HostError> {
        let source = params.document.source.clone();
        match &self.kind {
            Kind::InProcess { plugin, ctx } => {
                plugin.collect(params, ctx).await.map_err(|e| e.to_string())
            }
            Kind::Remote { peer, .. } => peer
                .request::<request::TransformCollect>(&params)
                .await
                .map_err(|e| e.to_string()),
        }
        .map_err(|message| self.phase_error("transform/collect", &source, message))
    }

    pub async fn apply(
        &self,
        params: TransformApplyParams,
    ) -> Result<TransformApplyResult, HostError> {
        let source = params.document.source.clone();
        match &self.kind {
            Kind::InProcess { plugin, ctx } => {
                plugin.apply(params, ctx).await.map_err(|e| e.to_string())
            }
            Kind::Remote { peer, .. } => peer
                .request::<request::TransformApply>(&params)
                .await
                .map_err(|e| e.to_string()),
        }
        .map_err(|message| self.phase_error("transform/apply", &source, message))
    }

    pub async fn render(&self, params: RenderParams) -> Result<RenderResult, HostError> {
        let source = params.document.source.clone();
        match &self.kind {
            Kind::InProcess { plugin, ctx } => {
                plugin.render(params, ctx).await.map_err(|e| e.to_string())
            }
            Kind::Remote { peer, .. } => peer
                .request::<request::Render>(&params)
                .await
                .map_err(|e| e.to_string()),
        }
        .map_err(|message| self.phase_error("render", &source, message))
    }

    /// Stops the plugin: `shutdown`, `exit`, then waits for the process,
    /// killing it if it does not exit in time.
    pub async fn shutdown(self) -> Result<(), HostError> {
        let Kind::Remote {
            peer,
            connection,
            mut child,
        } = self.kind
        else {
            return Ok(());
        };
        let name = self.name;
        let polite = async {
            peer.request::<request::Shutdown>(&()).await?;
            peer.notify::<notification::Exit>(&()).await?;
            Ok::<_, lsdocs_rpc::Error>(())
        };
        if let Err(e) = tokio::time::timeout(EXIT_TIMEOUT, polite)
            .await
            .unwrap_or(Err(lsdocs_rpc::Error::Timeout))
        {
            tracing::warn!(plugin = %name, "shutdown failed: {e}");
        }
        peer.close();
        let _ = tokio::time::timeout(EXIT_TIMEOUT, connection).await;
        match tokio::time::timeout(EXIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(Ok(status)) => {
                tracing::warn!(plugin = %name, "exited with {status}");
                Ok(())
            }
            Ok(Err(e)) => Err(HostError::launch(&name, e)),
            Err(_) => {
                tracing::warn!(plugin = %name, "did not exit in time; killing it");
                let _ = child.kill().await;
                Ok(())
            }
        }
    }

    fn phase_error(&self, phase: &str, source: &str, message: String) -> HostError {
        HostError::Plugin {
            plugin: self.name.clone(),
            phase: phase.into(),
            source_file: source.into(),
            message,
        }
    }
}

fn compile_globs(plugin: &str, patterns: &[String]) -> Result<Option<GlobSet>, HostError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|e| {
            HostError::Config(format!("plugin `{plugin}`: bad `files` pattern: {e}"))
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| HostError::Config(format!("plugin `{plugin}`: {e}")))
}

/// Builds the process command for a plugin. Relative program paths are
/// resolved against the site root, as the child's working directory does
/// not apply to finding the program itself on every platform.
fn command_for(command: &Command, root: &Path) -> tokio::process::Command {
    let resolve = |program: &str| {
        let path = Path::new(program);
        if path.is_relative() && path.components().count() > 1 {
            root.join(path).into_os_string()
        } else {
            program.into()
        }
    };
    match command {
        Command::Program { program, args } => {
            let mut cmd = tokio::process::Command::new(resolve(program));
            cmd.args(args);
            cmd
        }
        Command::Python { module } => {
            let python = std::env::var("LSDOCS_PYTHON")
                .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.to_owned());
            let mut cmd = tokio::process::Command::new(python);
            cmd.args(["-m", module]);
            cmd
        }
        Command::Node { entry } => {
            let node = std::env::var("LSDOCS_NODE").unwrap_or_else(|_| "node".to_owned());
            let mut cmd = tokio::process::Command::new(node);
            cmd.arg(root.join(entry));
            cmd
        }
    }
}

fn spawn(plugin: &str, cmd: &mut tokio::process::Command) -> Result<Child, HostError> {
    cmd.spawn().map_err(|e| HostError::launch(plugin, e))
}

/// Routes a remote plugin's notifications to [`Events`].
fn router(plugin: &str, events: &Events) -> Router {
    let (log_name, log_events) = (plugin.to_owned(), events.clone());
    let (progress_name, progress_events) = (plugin.to_owned(), events.clone());
    let (diag_name, diag_events) = (plugin.to_owned(), events.clone());
    Router::new()
        .notification::<notification::Log, _>(move |p, _| {
            log_events.handle(&log_name, Event::Log(p))
        })
        .notification::<notification::Progress, _>(move |p, _| {
            progress_events.handle(&progress_name, Event::Progress(p))
        })
        .notification::<notification::Diagnostics, _>(move |p, _| {
            diag_events.handle(&diag_name, Event::Diagnostics(p))
        })
}
