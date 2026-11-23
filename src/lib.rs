//! rustocop — a fast Ruby linter and autocorrector, compatible with RuboCop.
//!
//! It runs RuboCop's cops with RuboCop's names, disable comments and
//! corrections, configured via `rustocop.toml`, and needs no Ruby runtime:
//! parsing is done through [Prism](https://github.com/ruby/prism), Ruby's
//! official parser, via its Rust bindings.

#[cfg(test)]
#[macro_use]
#[path = "../tests/support/parallel_rows.rs"]
mod parallel_rows;

#[cfg(test)]
#[path = "../tests/support/fixture.rs"]
mod fixture;

pub mod baseline;
pub mod config;
pub mod cops;
pub mod corrections;
pub mod directives;
pub mod discovery;
pub mod import;
pub mod isolation;
pub mod offense;
pub mod output;
mod parser;
pub mod pipeline;
pub mod source;
