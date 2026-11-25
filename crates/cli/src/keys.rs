//! Single-key commands for `lsdocs serve` (`r` rebuild, `d` debug output,
//! `h` help, `q` quit), read on a thread of their own.
//!
//! Keys are only read when stdin and stderr are both a terminal, so piped
//! and scripted runs (and the tests) behave as before. On Unix the
//! terminal is put in "cbreak" mode: no line buffering and no echo, but
//! Ctrl+C still interrupts and output is still processed, so log lines,
//! plugins' own stderr included, keep their line endings. On Windows
//! crossterm's raw mode changes only the console's input mode, which has
//! the same effect, except that Ctrl+C arrives as a key.

use std::io::IsTerminal;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;

/// A command given with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Restart the plugins and rebuild.
    Rebuild,
    /// Switch debug output on or off.
    Debug,
    /// Show the keys.
    Help,
    /// Stop serving.
    Quit,
}

impl Key {
    /// The command for a key press, if any.
    fn from_event(event: KeyEvent) -> Option<Self> {
        if event.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        match event.code {
            KeyCode::Char('c' | 'd') if ctrl => Some(Self::Quit),
            _ if ctrl => None,
            KeyCode::Char('r' | 'R') => Some(Self::Rebuild),
            KeyCode::Char('d' | 'D') => Some(Self::Debug),
            KeyCode::Char('h' | 'H' | '?') => Some(Self::Help),
            KeyCode::Char('q' | 'Q') => Some(Self::Quit),
            _ => None,
        }
    }
}

/// The help `h` shows; `debug` is whether debug output is on.
pub fn help(debug: bool) -> String {
    let debug = if debug { "on" } else { "off" };
    format!(
        "keys:\n  \
         r  restart plugins and rebuild (reloads config and theme)\n  \
         d  debug output (now {debug})\n  \
         h  show this help\n  \
         q  quit (or Ctrl+C)\n"
    )
}

/// Key commands as they are pressed. Dropping it restores the terminal.
pub struct Keys {
    rx: mpsc::UnboundedReceiver<Key>,
    _mode: Mode,
}

impl Keys {
    /// Starts reading keys, if stdin and stderr are a terminal and its
    /// mode can be set.
    pub fn start() -> Option<Self> {
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return None;
        }
        let mode = match Mode::enter() {
            Ok(mode) => mode,
            Err(e) => {
                tracing::debug!("keys are off: cannot set the terminal mode: {e}");
                return None;
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let reader = std::thread::Builder::new()
            .name("lsdocs-keys".into())
            .spawn(move || {
                loop {
                    match crossterm::event::read() {
                        Ok(Event::Key(event)) => {
                            if let Some(key) = Key::from_event(event)
                                && tx.send(key).is_err()
                            {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::debug!("keys are off: {e}");
                            break;
                        }
                    }
                }
            });
        if let Err(e) = reader {
            tracing::debug!("keys are off: {e}");
            return None;
        }
        Some(Self { rx, _mode: mode })
    }

    /// The next command; `None` once keys can no longer be read.
    pub async fn next(&mut self) -> Option<Key> {
        self.rx.recv().await
    }
}

/// The terminal's mode while keys are read; restored on drop.
#[cfg(unix)]
struct Mode {
    saved: rustix::termios::Termios,
}

#[cfg(unix)]
impl Mode {
    fn enter() -> std::io::Result<Self> {
        use rustix::termios::{LocalModes, OptionalActions, SpecialCodeIndex};
        let stdin = std::io::stdin();
        let saved = rustix::termios::tcgetattr(&stdin)?;
        let mut mode = saved.clone();
        mode.local_modes
            .remove(LocalModes::ICANON | LocalModes::ECHO);
        mode.special_codes[SpecialCodeIndex::VMIN] = 1;
        mode.special_codes[SpecialCodeIndex::VTIME] = 0;
        rustix::termios::tcsetattr(&stdin, OptionalActions::Now, &mode)?;
        Ok(Self { saved })
    }
}

#[cfg(unix)]
impl Drop for Mode {
    fn drop(&mut self) {
        let stdin = std::io::stdin();
        let _ =
            rustix::termios::tcsetattr(&stdin, rustix::termios::OptionalActions::Now, &self.saved);
    }
}

#[cfg(not(unix))]
struct Mode;

#[cfg(not(unix))]
impl Mode {
    fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

#[cfg(not(unix))]
impl Drop for Mode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
        Key::from_event(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn keys_map_to_commands() {
        let none = KeyModifiers::NONE;
        assert_eq!(press(KeyCode::Char('r'), none), Some(Key::Rebuild));
        assert_eq!(press(KeyCode::Char('d'), none), Some(Key::Debug));
        assert_eq!(press(KeyCode::Char('?'), none), Some(Key::Help));
        assert_eq!(press(KeyCode::Char('q'), none), Some(Key::Quit));
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(press(KeyCode::Char('c'), ctrl), Some(Key::Quit));
        assert_eq!(press(KeyCode::Char('r'), ctrl), None);
        assert_eq!(press(KeyCode::Char('x'), none), None);
        // Releases (reported on Windows) are not presses.
        let mut release = KeyEvent::new(KeyCode::Char('q'), none);
        release.kind = KeyEventKind::Release;
        assert_eq!(Key::from_event(release), None);
    }
}
