//! The RuboCop spec oracle (`tools/oracle`): RuboCop's
//! `expect_offense` / `expect_no_offenses` examples for every registered cop,
//! run against the `rustocop` binary. Every deviation must be a known one in
//! `tools/oracle/baseline.txt`.
//!
//! Runs locally only: it needs Ruby, a RuboCop source checkout
//! (`$RUBOCOP_SRC` or `~/.cache/rubocop`) and RuboCop's runtime gems, and
//! skips (passing, with a note on stderr) without them.

use std::{
    path::Path,
    process::{Command, Output},
};

use rustocop::cops::Registry;

/// The oracle's exit status when its prerequisites are missing.
const EXIT_UNAVAILABLE: i32 = 3;

/// Run a Ruby script under `tools/oracle`; `None` when the oracle cannot run
/// here.
fn ruby(script: &str, args: &[&str]) -> Option<Output> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/oracle")
        .join(script);
    let output = match Command::new("ruby").arg(path).args(args).output() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("skipping the RuboCop spec oracle: cannot run ruby ({err})");
            return None;
        }
    };
    if output.status.code() == Some(EXIT_UNAVAILABLE) {
        eprintln!(
            "skipping the RuboCop spec oracle: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    Some(output)
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn oracle_discovery_passes_offline_tests() {
    if let Some(output) = ruby("test/discovery_test.rb", &[]) {
        assert_success(&output);
    }
}

#[test]
fn fixed_reference_policy_passes_offline_tests() {
    if let Some(output) = ruby("../test/fixed_policy_test.rb", &[]) {
        assert_success(&output);
    }
}

#[test]
fn oracle_machinery_passes_its_own_tests() {
    if let Some(output) = ruby("test/oracle_test.rb", &[]) {
        assert_success(&output);
    }
}

#[test]
fn rubocop_spec_examples_match_or_are_known_deviations() {
    let registry = Registry::new();
    let mut args = vec![
        "--pinned",
        "--quiet",
        "--binary",
        env!("CARGO_BIN_EXE_rustocop"),
    ];
    args.extend(registry.cops().iter().map(|cop| cop.name()));
    if let Some(output) = ruby("oracle.rb", &args) {
        assert_success(&output);
    }
}
