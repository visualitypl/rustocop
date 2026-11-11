//! Bundler conventions for explicitly supplied inputs.
pub(crate) mod declarations;
mod duplicated_gem;
mod duplicated_group;
mod gem_comment;
mod gem_filename;
mod gem_version;
mod insecure_protocol_source;
pub(crate) mod ordered_gem_node;
mod ordered_gems;
pub use duplicated_gem::DuplicatedGem;
pub use duplicated_group::DuplicatedGroup;
pub use gem_comment::GemComment;
pub use gem_filename::GemFilename;
pub use gem_version::GemVersion;
pub use insecure_protocol_source::InsecureProtocolSource;
pub use ordered_gems::OrderedGems;

#[cfg(test)]
pub(super) mod tests {
    use crate::{config::ResolvedConfig, cops::Cop, offense::Offense, pipeline::lint_one};
    use std::path::Path;

    /// Lint `source` as the supplied `path` with only `cop` selected.
    pub(crate) fn lint(cop: &dyn Cop, path: &str, source: &str) -> Vec<Offense> {
        let config = ResolvedConfig::default();
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(source.as_bytes()),
            &[cop],
            &config,
        );
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.internal_errors.is_empty(), "{report:?}");
        report.result.offenses
    }

    /// Exact sorted (line, column, message) findings, never a correction.
    pub(crate) fn assert_offenses(
        cop: &dyn Cop,
        path: &str,
        source: &str,
        expected: &[(u32, u32, String)],
    ) {
        let mut found: Vec<_> = lint(cop, path, source)
            .into_iter()
            .map(|o| {
                assert_eq!(o.cop, cop.name());
                assert_eq!(o.end_line, o.line);
                (o.line, o.col, o.message)
            })
            .collect();
        found.sort();
        let mut expected = expected.to_vec();
        expected.sort();
        assert_eq!(found, expected, "{source}");
    }
}
