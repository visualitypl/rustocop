//! The Ruby that runtime tests run. `RUSTOCOP_TEST_RUBY` selects it by
//! version: `3.4` (the default) reads the path in `RUSTOCOP_RUBY34`, and `4.0`
//! reads the path in `RUSTOCOP_RUBY40`. A test that is valid on Ruby 3.4 only
//! starts with `if runtime::skips("reason") { return; }`.

use std::{
    env::{self, VarError},
    ffi::OsString,
};

pub const SELECTOR: &str = "RUSTOCOP_TEST_RUBY";

fn is_ruby_4_0() -> bool {
    match env::var(SELECTOR).as_deref() {
        Ok("4.0") => true,
        Ok("3.4") | Err(VarError::NotPresent) => false,
        other => panic!("{SELECTOR} must be 3.4 or 4.0, not {other:?}"),
    }
}

fn path_variable() -> &'static str {
    if is_ruby_4_0() {
        "RUSTOCOP_RUBY40"
    } else {
        "RUSTOCOP_RUBY34"
    }
}

pub fn ruby() -> Result<String, VarError> {
    env::var(path_variable())
}

pub fn ruby_os() -> Option<OsString> {
    env::var_os(path_variable())
}

pub fn version_prefix() -> &'static str {
    if is_ruby_4_0() { "4.0." } else { "3.4." }
}

pub fn banner_prefix() -> &'static str {
    if is_ruby_4_0() {
        "ruby 4.0."
    } else {
        "ruby 3.4."
    }
}

pub fn script(source: &str) -> String {
    assert!(source.contains("'3.4.'"), "{source}");
    source.replace("'3.4.'", &format!("'{}'", version_prefix()))
}

pub fn skips(reason: &str) -> bool {
    let skipped = is_ruby_4_0();
    if skipped {
        eprintln!("SKIP on Ruby 4.0: {reason}");
    }
    skipped
}
