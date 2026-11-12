//! Contract for the std-only matrix runner used by slow fixture tests.
use crate::support::parallel_rows;

use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn rows_run_once_with_bounded_real_parallelism() {
    let barrier = Barrier::new(parallel_rows::WORKERS);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let seen = Mutex::new(vec![0; 24]);
    parallel_rows!(for row in 0..24 => {
        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(current, Ordering::SeqCst);
        // Every worker must run concurrently; repeat to exercise reuse.
        barrier.wait();
        seen.lock().unwrap()[row] += 1;
        active.fetch_sub(1, Ordering::SeqCst);
        barrier.wait();
    });
    assert_eq!(peak.load(Ordering::SeqCst), parallel_rows::WORKERS);
    assert_eq!(*seen.lock().unwrap(), vec![1; 24]);
}

#[test]
fn failures_name_rows_keep_assertion_details_and_finish_other_rows() {
    let seen = Arc::new(AtomicUsize::new(0));
    let result = std::panic::catch_unwind(|| {
        parallel_rows!(for row in ["good", "broken fixture", "also good"] => {
            seen.fetch_add(1, Ordering::SeqCst);
            assert_ne!(row, "broken fixture", "original assertion details");
        });
    });
    let payload = result.unwrap_err();
    let message = payload.downcast_ref::<String>().unwrap();
    assert!(message.contains("row 1: \"broken fixture\""), "{message}");
    assert!(message.contains("original assertion details"), "{message}");
    assert_eq!(seen.load(Ordering::SeqCst), 3);
}

#[test]
fn empty_short_borrowed_rows_and_continue_keep_loop_semantics() {
    let empty_visits = AtomicUsize::new(0);
    parallel_rows!(for _row in Vec::<usize>::new() => {
        empty_visits.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(empty_visits.load(Ordering::SeqCst), 0);
    let rows = ["skip".to_owned(), "keep".to_owned()];
    let seen = Mutex::new(Vec::new());
    parallel_rows!(for row in &rows => {
        if row == "skip" {
            continue;
        }
        seen.lock().unwrap().push(row.clone());
    });
    assert_eq!(*seen.lock().unwrap(), ["keep"]);
}
