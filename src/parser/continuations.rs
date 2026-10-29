//! Narrow lexer evidence, collected during the one main parse. No token stream,
//! alternate parse, AST walk, or Ruby subprocess is constructed here.
use std::ffi::c_void;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Continuation {
    pub offset: usize,
    pub previous_end: usize,
    /// The main lexer resumes after queued heredoc bodies, not inside them.
    pub following_line: usize,
    pub ignored: bool,
}

type Callback = unsafe extern "C" fn(*mut c_void, usize, usize, usize, bool);
unsafe extern "C" {
    fn pm_rustocop_continuation_callback_set(callback: Option<Callback>, data: *mut c_void);
}

/// Scoped on the parsing thread (including stacker's same-thread segment).
/// The C callback is observation-only and cannot unwind. Clearing it on drop
/// prevents a dangling pointer after a parse or Rust-side diagnostic unwind.
struct Observer;
impl Drop for Observer {
    fn drop(&mut self) {
        unsafe { pm_rustocop_continuation_callback_set(None, std::ptr::null_mut()) };
    }
}
unsafe extern "C" fn record(
    data: *mut c_void,
    offset: usize,
    previous_end: usize,
    following_line: usize,
    ignored: bool,
) {
    // SAFETY: `collect` keeps this Vec alive and exclusively borrowed until the
    // callback is removed; Prism invokes it only synchronously on this thread.
    let records = unsafe { &mut *data.cast::<Vec<Continuation>>() };
    records.push(Continuation {
        offset,
        previous_end,
        following_line,
        ignored,
    });
}

pub(super) fn collect<T>(parse: impl FnOnce() -> T) -> (T, Vec<Continuation>) {
    let mut records = Vec::new();
    unsafe {
        pm_rustocop_continuation_callback_set(Some(record), (&raw mut records).cast());
    }
    let guard = Observer;
    let result = parse();
    drop(guard);
    (result, records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_code_only_and_retains_main_lexer_newline_state() {
        let source = b"# comment \\\nfoo(1, \\\n 'text \\\n #{literal}')\nfoo \\\n + 2\n/x \\\n y/\n%w[a \\\n b]\n`never \\\n executed`\nx = <<~TXT\nbody \\\ntext\nTXT\n__END__\ndata \\\n";
        let parsed = crate::parser::parse(source);
        assert_eq!(parsed.errors().count(), 0);
        assert_eq!(parsed.continuations.len(), 2);
        let first = parsed.continuations[0];
        let second = parsed.continuations[1];
        assert!(first.ignored);
        assert!(!second.ignored);
        assert_eq!(&source[first.previous_end - 1..first.previous_end], b",");
        for record in &parsed.continuations {
            assert_eq!(&source[record.offset..record.offset + 2], b"\\\n");
        }
    }

    #[test]
    fn observer_clears_on_unwind_and_is_thread_local() {
        assert!(std::panic::catch_unwind(|| collect(|| panic!("before parsing"))).is_err());
        let threads: Vec<_> = (1..=8)
            .map(|count| {
                std::thread::spawn(move || {
                    let source = "foo(1, \\\n 2)\n".repeat(count);
                    let parsed = crate::parser::parse(source.as_bytes());
                    assert_eq!(parsed.continuations.len(), count);
                    assert!(parsed.continuations.iter().all(|r| r.ignored));
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(crate::parser::parse(b"foo\n").continuations.is_empty());
    }

    #[test]
    fn legacy_multibyte_trailing_backslash_is_not_a_continuation() {
        for source in [
            b"\xef\xbb\xbffoo(1, \\\r\n 2)\r\n".as_slice(),
            b"# encoding: Windows-31J\nx = '\x83\\\n'\nfoo(1, \\\n 2)\n",
        ] {
            let parsed = crate::parser::parse(source);
            assert_eq!(parsed.errors().count(), 0);
            assert_eq!(parsed.continuations.len(), 1);
            assert!(parsed.continuations[0].ignored);
        }
    }
}
