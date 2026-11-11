//! Bounded, std-only row parallelism. Each caller owns its row-local files.
use std::{
    collections::VecDeque,
    fmt::Debug,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Mutex,
    thread,
};

// Nextest already runs twelve tests at once. Never create a thread per row.
pub const WORKERS: usize = 4;

pub fn for_each<T: Send + Debug>(rows: impl IntoIterator<Item = T>, check: impl Fn(T) + Sync) {
    let rows: VecDeque<_> = rows.into_iter().enumerate().collect();
    let workers = WORKERS.min(rows.len());
    let pending = Mutex::new(rows);
    let failures = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let Some((index, row)) = pending.lock().unwrap().pop_front() else {
                        break;
                    };
                    // Preserve the named fixture (or source tuple) before moving it into check.
                    let label = format!("{row:?}");
                    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| check(row))) {
                        let message = payload
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| payload.downcast_ref::<&str>().copied())
                            .unwrap_or("non-string panic payload");
                        failures
                            .lock()
                            .unwrap()
                            .push((index, format!("row {index}: {label}\n{message}")));
                    }
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort_by_key(|(index, _)| *index);
    assert!(
        failures.is_empty(),
        "{}",
        failures
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// Retain `continue` and nested-loop semantics in existing fixture assertions.
macro_rules! parallel_rows {
    (for $row:pat in $rows:expr => $body:block) => {
        $crate::parallel_rows::for_each($rows, |value| {
            for $row in std::iter::once(value) $body
        })
    };
}
