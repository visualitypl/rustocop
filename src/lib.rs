//! rustocop — a blazing-fast, opinionated Ruby linter.
//!
//! Zero-config by default, configured via `rustocop.toml`, and requiring no
//! Ruby runtime: parsing is done through [Prism](https://github.com/ruby/prism),
//! Ruby's official parser, via its Rust bindings.

#[cfg(test)]
#[macro_use]
#[path = "../tests/support/parallel_rows.rs"]
mod parallel_rows;

pub mod config;
pub mod cops;
pub mod corrections;
pub mod directives;
pub mod discovery;
pub mod isolation;
pub mod offense;
pub mod output;
mod parser;
pub mod pipeline;
pub mod source;
