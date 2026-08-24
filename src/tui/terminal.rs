use std::io::{self, Stderr};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(unix)]
use std::sync::{Arc, OnceLock};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

#[cfg(unix)]
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::flag;

use super::PickerSignal;

type PickerTerminal = Terminal<CrosstermBackend<Stderr>>;

/// Owns every terminal mutation and reverses initialized phases on all normal
/// returns, I/O errors, and unwinds. Signal handlers only update atomics; the
/// event loop performs restoration in ordinary Rust code.
pub(super) struct TerminalSession {
    terminal: Option<PickerTerminal>,
    phases: TerminalPhases,
    signal_lease: SignalLease,
}

#[derive(Default)]
struct TerminalPhases {
    raw_mode_maybe_enabled: bool,
    alternate_screen_maybe_entered: bool,
    cursor_maybe_hidden: bool,
}

impl TerminalSession {
    pub(super) fn start() -> io::Result<Self> {
        let signal_lease = SignalLease::acquire()?;
        let mut session = Self {
            terminal: None,
            phases: TerminalPhases::default(),
            signal_lease,
        };

        // Set phase flags before each fallible operation. Cleanup is harmless
        // when an operation failed before changing the terminal and essential
        // when it changed the terminal but failed while flushing.
        session.phases.raw_mode_maybe_enabled = true;
        enable_raw_mode()?;

        let mut stderr = io::stderr();
        session.phases.alternate_screen_maybe_entered = true;
        execute!(stderr, EnterAlternateScreen)?;

        session.phases.cursor_maybe_hidden = true;
        execute!(stderr, Hide)?;

        let backend = CrosstermBackend::new(stderr);
        session.terminal = Some(Terminal::new(backend)?);
        Ok(session)
    }

    pub(super) fn terminal_mut(&mut self) -> io::Result<&mut PickerTerminal> {
        self.terminal
            .as_mut()
            .ok_or_else(|| io::Error::other("terminal session is unavailable after initialization"))
    }

    pub(super) fn take_signal(&self) -> Option<PickerSignal> {
        self.signal_lease.take_signal()
    }

    pub(super) fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;
        let mut stderr = io::stderr();

        if self.phases.cursor_maybe_hidden {
            let result = execute!(stderr, Show);
            if result.is_ok() {
                self.phases.cursor_maybe_hidden = false;
            }
            record_first_error(&mut first_error, result);
        }

        if self.phases.alternate_screen_maybe_entered {
            let result = execute!(stderr, LeaveAlternateScreen);
            if result.is_ok() {
                self.phases.alternate_screen_maybe_entered = false;
            }
            record_first_error(&mut first_error, result);
        }

        if self.phases.raw_mode_maybe_enabled {
            let result = disable_raw_mode();
            if result.is_ok() {
                self.phases.raw_mode_maybe_enabled = false;
            }
            record_first_error(&mut first_error, result);
        }

        // Restore default signal behavior only after terminal restoration. On
        // failure, Drop retries the still-active phases before releasing it.
        if first_error.is_none() {
            self.signal_lease.release();
        }

        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn record_first_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if first_error.is_none()
        && let Err(error) = result
    {
        *first_error = Some(error);
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct SignalRegistry {
    pending: Arc<AtomicUsize>,
    active: Arc<AtomicBool>,
    emulate_default: Arc<AtomicBool>,
}

#[cfg(unix)]
impl SignalRegistry {
    fn install() -> io::Result<Self> {
        let registry = Self {
            pending: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicBool::new(false)),
            // Outside an active TUI, registered handlers emulate the normal
            // default action instead of leaving termination signals ignored.
            emulate_default: Arc::new(AtomicBool::new(true)),
        };

        registry.register(SIGINT)?;
        registry.register(SIGTERM)?;
        registry.register(SIGHUP)?;
        Ok(registry)
    }

    fn register(&self, signal: i32) -> io::Result<()> {
        // Registration order matters: once inactive, the default action runs
        // before another action can swallow the signal.
        flag::register_conditional_default(signal, Arc::clone(&self.emulate_default))?;
        let signal_value = usize::try_from(signal).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid negative signal number",
            )
        })?;
        flag::register_usize(signal, Arc::clone(&self.pending), signal_value)?;
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct StoredIoError {
    kind: io::ErrorKind,
    message: String,
}

#[cfg(unix)]
impl StoredIoError {
    fn capture(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    fn to_io_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

#[cfg(unix)]
static SIGNAL_REGISTRY: OnceLock<Result<SignalRegistry, StoredIoError>> = OnceLock::new();

#[cfg(unix)]
struct SignalLease {
    registry: SignalRegistry,
    released: bool,
}

#[cfg(unix)]
impl SignalLease {
    fn acquire() -> io::Result<Self> {
        let registry = match SIGNAL_REGISTRY.get_or_init(|| {
            SignalRegistry::install().map_err(|error| StoredIoError::capture(&error))
        }) {
            Ok(registry) => registry.clone(),
            Err(error) => return Err(error.to_io_error()),
        };

        if registry
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another terminal picker is already active in this process",
            ));
        }

        registry.pending.store(0, Ordering::SeqCst);
        registry.emulate_default.store(false, Ordering::SeqCst);
        Ok(Self {
            registry,
            released: false,
        })
    }

    fn take_signal(&self) -> Option<PickerSignal> {
        let signal = self.registry.pending.swap(0, Ordering::SeqCst);
        if signal == SIGINT as usize {
            Some(PickerSignal::Interrupt)
        } else if signal == SIGTERM as usize {
            Some(PickerSignal::Terminate)
        } else {
            if signal == SIGHUP as usize {
                return Some(PickerSignal::Hangup);
            }
            None
        }
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        self.registry.emulate_default.store(true, Ordering::SeqCst);
        self.registry.active.store(false, Ordering::SeqCst);
        self.released = true;
    }
}

#[cfg(unix)]
impl Drop for SignalLease {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(not(unix))]
struct SignalLease;

#[cfg(not(unix))]
impl SignalLease {
    fn acquire() -> io::Result<Self> {
        Ok(Self)
    }

    fn take_signal(&self) -> Option<PickerSignal> {
        None
    }

    fn release(&mut self) {}
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn signal_lease(signal: i32) -> SignalLease {
        let pending = usize::try_from(signal).unwrap();
        SignalLease {
            registry: SignalRegistry {
                pending: Arc::new(AtomicUsize::new(pending)),
                active: Arc::new(AtomicBool::new(true)),
                emulate_default: Arc::new(AtomicBool::new(false)),
            },
            released: false,
        }
    }

    #[test]
    fn signal_numbers_map_to_semantic_values() {
        let interrupt = signal_lease(SIGINT);
        assert_eq!(interrupt.take_signal(), Some(PickerSignal::Interrupt));
        assert_eq!(interrupt.take_signal(), None);

        let terminate = signal_lease(SIGTERM);
        assert_eq!(terminate.take_signal(), Some(PickerSignal::Terminate));

        let hangup = signal_lease(SIGHUP);
        assert_eq!(hangup.take_signal(), Some(PickerSignal::Hangup));
    }
}
