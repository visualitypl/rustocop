//! Panic isolation: a bug in one cop on one file must not abort the run.
//!
//! The pipeline runs every cop call under [`catch`], inside a [`QuietPanics`]
//! scope. While a thread is in such a scope, the panic hook records the
//! panic's source location (and, when `RUST_BACKTRACE` asks for one, a
//! backtrace) for the catcher instead of printing the default
//! `thread '…' panicked at …` message. The pipeline then reports an internal
//! error naming the cop and file, which carries everything that message did.
//! Panics on threads outside a scope (test assertions, the CLI itself) go to
//! the previous hook unchanged.
//!
//! Isolation needs `panic = "unwind"`, Cargo's default: under
//! `panic = "abort"` nothing can be caught.

use std::{
    any::Any,
    backtrace::{Backtrace, BacktraceStatus},
    cell::{Cell, RefCell},
    marker::PhantomData,
    panic::{self, AssertUnwindSafe, PanicHookInfo},
    sync::Once,
};

/// A caught panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaughtPanic {
    /// The panic message, e.g. `index out of bounds: the len is 1 but the
    /// index is 1`.
    pub message: String,
    /// Where the panic was raised, as `file:line:col`. Known only for panics
    /// inside a [`QuietPanics`] scope.
    pub location: Option<String>,
    /// A backtrace, when the panic was inside a [`QuietPanics`] scope and
    /// `RUST_BACKTRACE` (or `RUST_LIB_BACKTRACE`) enabled capturing.
    pub backtrace: Option<String>,
}

/// Run `f`, catching a panic instead of unwinding past the caller.
///
/// Call it inside a [`QuietPanics`] scope to get the panic's location and
/// keep the default message quiet. The happy path touches no thread-local
/// state, so the walker can afford one per cop per node (about 1% of a
/// single-threaded run when introduced).
///
/// `f` is asserted unwind-safe: callers hand it the shared, read-only
/// `SourceFile` (whose lazy caches stay unset when an initializer panics)
/// and an append-only offense list, both of which stay valid after a panic.
pub fn catch<R>(f: impl FnOnce() -> R) -> Result<R, CaughtPanic> {
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| caught(&*payload))
}

/// Package a caught payload with what the hook recorded about it.
#[cold]
fn caught(payload: &(dyn Any + Send)) -> CaughtPanic {
    let record = LAST_PANIC
        .try_with(|slot| slot.take())
        .ok()
        .flatten()
        .unwrap_or_default();
    CaughtPanic {
        message: payload_message(payload),
        location: record.location,
        backtrace: record.backtrace,
    }
}

/// While alive, panics on this thread are recorded for [`catch`] instead of
/// being printed by the previous panic hook. Scopes nest; the guard is tied
/// to its thread.
pub struct QuietPanics {
    _not_send: PhantomData<*const ()>,
}

impl QuietPanics {
    /// Enter a scope, installing rustocop's panic hook on first use.
    pub fn enter() -> Self {
        install_hook();
        QUIET_DEPTH.with(|depth| depth.set(depth.get() + 1));
        // Nothing recorded before this scope belongs to a panic inside it.
        LAST_PANIC.with(|slot| slot.take());
        QuietPanics {
            _not_send: PhantomData,
        }
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        QUIET_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// Chain rustocop's hook in front of the current one, once per process.
fn install_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let quiet = QUIET_DEPTH.try_with(Cell::get).is_ok_and(|depth| depth > 0);
            if quiet {
                record(info);
            } else {
                previous(info);
            }
        }));
    });
}

/// Keep what the catcher cannot see: where the panic happened and, when
/// asked for, how it got there.
fn record(info: &PanicHookInfo<'_>) {
    let location = info
        .location()
        .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
    let backtrace = Backtrace::capture();
    let backtrace =
        (backtrace.status() == BacktraceStatus::Captured).then(|| backtrace.to_string());
    let _ = LAST_PANIC.try_with(|slot| {
        slot.replace(Some(HookRecord {
            location,
            backtrace,
        }))
    });
}

thread_local! {
    /// How many [`QuietPanics`] scopes this thread is in.
    static QUIET_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// What the hook recorded about this thread's latest quiet panic.
    static LAST_PANIC: RefCell<Option<HookRecord>> = const { RefCell::new(None) };
}

/// What only the panic hook can see.
#[derive(Debug, Default)]
struct HookRecord {
    location: Option<String>,
    backtrace: Option<String>,
}

/// The message of a panic payload, as the default hook prints it.
fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "Box<dyn Any>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_value_when_nothing_panics() {
        let _quiet = QuietPanics::enter();
        assert_eq!(catch(|| 40 + 2), Ok(42));
    }

    #[test]
    fn catches_a_formatted_panic_with_its_location() {
        let _quiet = QuietPanics::enter();
        let line = line!() + 1;
        let caught = catch(|| panic!("index {} is out of bounds", 3)).unwrap_err();
        assert_eq!(caught.message, "index 3 is out of bounds");
        let location = caught.location.expect("location recorded");
        assert!(
            location.starts_with(&format!("src/isolation.rs:{line}:")),
            "{location}"
        );
    }

    #[test]
    fn catches_a_static_str_panic() {
        let _quiet = QuietPanics::enter();
        let caught = catch(|| -> u8 { panic!("static message") }).unwrap_err();
        assert_eq!(caught.message, "static message");
    }

    #[test]
    fn catches_a_panic_from_an_index_out_of_bounds() {
        let _quiet = QuietPanics::enter();
        let items: Vec<u8> = vec![1];
        let index = items.len();
        let caught = catch(|| items[index]).unwrap_err();
        assert!(
            caught.message.contains("out of bounds"),
            "{}",
            caught.message
        );
        assert!(caught.location.is_some());
    }

    #[test]
    fn non_string_payloads_get_a_placeholder_message() {
        let _quiet = QuietPanics::enter();
        let caught = catch(|| panic::panic_any(7_u32)).unwrap_err();
        assert_eq!(caught.message, "Box<dyn Any>");
    }

    #[test]
    fn outside_a_scope_panics_are_caught_but_not_recorded() {
        // The previous hook prints this one (into the test's captured output).
        let caught = catch(|| panic!("loud")).unwrap_err();
        assert_eq!(caught.message, "loud");
        assert_eq!(caught.location, None);
        assert_eq!(caught.backtrace, None);
    }

    #[test]
    fn scopes_nest_and_end_with_their_guard() {
        let outer = QuietPanics::enter();
        {
            let _inner = QuietPanics::enter();
        }
        let caught = catch(|| panic!("still quiet")).unwrap_err();
        assert!(caught.location.is_some(), "outer scope still active");
        drop(outer);
        let caught = catch(|| panic!("loud again")).unwrap_err();
        assert_eq!(caught.location, None, "no scope left");
    }

    #[test]
    fn scopes_are_per_thread() {
        let _quiet = QuietPanics::enter();
        let caught = std::thread::spawn(|| catch(|| panic!("other thread")).unwrap_err())
            .join()
            .unwrap();
        assert_eq!(caught.location, None);
    }

    /// Run by `quiet_panics_print_nothing_and_others_stay_loud` in a child
    /// process, whose stderr shows what the hook printed.
    #[test]
    #[ignore = "run in a child process by quiet_panics_print_nothing_and_others_stay_loud"]
    fn child_panics_quietly_then_loudly() {
        {
            let _quiet = QuietPanics::enter();
            let caught = catch(|| panic!("quiet-panic-marker")).unwrap_err();
            eprintln!("quiet backtrace captured: {}", caught.backtrace.is_some());
        }
        catch(|| panic!("loud-panic-marker")).unwrap_err();
    }

    /// Run `child_panics_quietly_then_loudly` in a child process; its stderr.
    fn child_stderr(rust_backtrace: Option<&str>) -> String {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args([
            "isolation::tests::child_panics_quietly_then_loudly",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        child.env_remove("RUST_LIB_BACKTRACE");
        match rust_backtrace {
            Some(value) => child.env("RUST_BACKTRACE", value),
            None => child.env_remove("RUST_BACKTRACE"),
        };
        let output = child.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "child failed: {stderr}");
        stderr
    }

    #[test]
    fn quiet_panics_print_nothing_and_others_stay_loud() {
        let stderr = child_stderr(None);
        assert!(!stderr.contains("quiet-panic-marker"), "{stderr}");
        assert!(stderr.contains("loud-panic-marker"), "{stderr}");
        assert!(
            stderr.contains("quiet backtrace captured: false"),
            "{stderr}"
        );
    }

    #[test]
    fn rust_backtrace_records_a_backtrace_for_quiet_panics() {
        let stderr = child_stderr(Some("1"));
        assert!(!stderr.contains("quiet-panic-marker"), "{stderr}");
        assert!(
            stderr.contains("quiet backtrace captured: true"),
            "{stderr}"
        );
    }
}
