//! Logging: `tracing` to stderr, with a filter `lsdocs serve` can switch
//! to debug output and back while it runs.

use std::io::IsTerminal;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::Layer;
use tracing_subscriber::fmt::time::Uptime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// How much to log, from the command line.
#[derive(Debug, Clone, Copy)]
pub struct Verbosity {
    /// `-v` count.
    pub verbose: u8,
    /// `-q`: warnings and errors only.
    pub quiet: bool,
    /// `--trace-rpc`.
    pub trace_rpc: bool,
    /// The level when neither `-v` nor `-q` is given.
    pub default: &'static str,
}

/// Swaps the filter at run time.
struct Toggle {
    handle: reload::Handle<EnvFilter, Registry>,
    verbosity: Verbosity,
    debug: AtomicBool,
}

static TOGGLE: OnceLock<Toggle> = OnceLock::new();

/// Installs the global subscriber. `LSDOCS_LOG` overrides `verbosity`.
pub fn init(verbosity: Verbosity) {
    let (layer, handle) = reload::Layer::new(filter(verbosity, false));
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false);
    // Times (since start) only with `-v`: progress lines read better
    // without them.
    let fmt = if verbosity.verbose > 0 {
        fmt.with_timer(Uptime::default()).boxed()
    } else {
        fmt.without_time().boxed()
    };
    if tracing_subscriber::registry()
        .with(layer)
        .with(fmt)
        .try_init()
        .is_ok()
    {
        let _ = TOGGLE.set(Toggle {
            handle,
            verbosity,
            debug: AtomicBool::new(false),
        });
    }
}

/// The filter for `verbosity`, or at least debug output if `debug`.
fn filter(verbosity: Verbosity, debug: bool) -> EnvFilter {
    if !debug && let Ok(filter) = EnvFilter::try_from_env("LSDOCS_LOG") {
        return filter;
    }
    let level = match (verbosity.quiet, verbosity.verbose) {
        _ if debug && verbosity.verbose < 2 => "debug",
        (true, 0) => "warn",
        (_, 0) => verbosity.default,
        (_, 1) => "debug",
        _ => "trace",
    };
    let mut filter = EnvFilter::new(level);
    // Protocol messages are logged at trace level; only `--trace-rpc`
    // shows them.
    let rpc = if verbosity.trace_rpc { "trace" } else { "off" };
    if let Ok(directive) = format!("{}={rpc}", lsdocs_plugin_sdk::TRACE_TARGET).parse() {
        filter = filter.add_directive(directive);
    }
    filter
}

/// Switches debug output on or off; returns whether it is now on.
pub fn toggle_debug() -> bool {
    let Some(toggle) = TOGGLE.get() else {
        return false;
    };
    let on = !toggle.debug.fetch_xor(true, Ordering::SeqCst);
    if let Err(e) = toggle.handle.reload(filter(toggle.verbosity, on)) {
        tracing::warn!("cannot change the log level: {e}");
    }
    on
}

/// Whether debug output was switched on with [`toggle_debug`].
pub fn debug_on() -> bool {
    TOGGLE.get().is_some_and(|t| t.debug.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::filter::LevelFilter;

    use super::*;

    fn verbosity(verbose: u8, quiet: bool) -> Verbosity {
        Verbosity {
            verbose,
            quiet,
            trace_rpc: false,
            default: "info",
        }
    }

    #[test]
    fn levels_follow_the_flags() {
        // `LSDOCS_LOG` replaces the flags.
        if std::env::var_os("LSDOCS_LOG").is_some() {
            return;
        }
        let level = |v: Verbosity, debug: bool| filter(v, debug).max_level_hint();
        assert_eq!(level(verbosity(0, false), false), Some(LevelFilter::INFO));
        assert_eq!(level(verbosity(0, true), false), Some(LevelFilter::WARN));
        assert_eq!(level(verbosity(1, false), false), Some(LevelFilter::DEBUG));
        assert_eq!(level(verbosity(2, false), false), Some(LevelFilter::TRACE));
        // The debug toggle raises the level, and never lowers it.
        assert_eq!(level(verbosity(0, true), true), Some(LevelFilter::DEBUG));
        assert_eq!(level(verbosity(2, false), true), Some(LevelFilter::TRACE));
    }
}
