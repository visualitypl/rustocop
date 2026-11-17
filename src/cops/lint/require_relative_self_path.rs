//! `Lint/RequireRelativeSelfPath` — a supplied Ruby filename requiring itself.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RequireRelativeSelfPath;
impl Cop for RequireRelativeSelfPath {
    fn name(&self) -> &'static str {
        "Lint/RequireRelativeSelfPath"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation() || call.name().as_slice() != b"require_relative" {
            return;
        }
        let path = file.source_path().as_os_str().as_encoded_bytes();
        // Compare supplied provenance bytes only. Never canonicalize, resolve,
        // read or load a required feature (even ./foo and ../foo differ).
        let path = &path[..path.iter().rposition(|b| *b != b'/').map_or(0, |i| i + 1)];
        let basename = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
        let Some(stem) = basename.strip_suffix(b".rb").filter(|s| !s.is_empty()) else {
            return;
        };
        let Some(argument) = call.arguments().and_then(|a| a.arguments().iter().next()) else {
            return;
        };
        let Some(value) = string_value(&argument, file) else {
            return;
        };
        // The supplied filename is UTF-8. Ruby compares ASCII-only strings
        // across compatible encodings, but equal non-ASCII bytes in a binary
        // reference literal are not the same string. The reference buffer keeps
        // binary literals binary even for Prism's forced-UTF8 Unicode escapes.
        if (value.binary && !value.bytes.is_ascii())
            || (value.bytes != stem && value.bytes != basename)
        {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: crate::cops::style::correction::whole_lines(
                        file,
                        node.location().start_offset()..crate::cops::ast::source_end(node),
                    ),
                    replacement: String::new(),
                }),
            line,
            col,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            message: "Remove the `require_relative` that requires itself.".into(),
        });
    }
}
struct StringValue {
    bytes: Vec<u8>,
    binary: bool,
}
fn string_value(node: &Node<'_>, file: &SourceFile) -> Option<StringValue> {
    if node.as_source_file_node().is_some() {
        // __FILE__ retains supplied path encoding, not the source's encoding.
        return Some(StringValue {
            bytes: file.source_path().as_os_str().as_encoded_bytes().to_vec(),
            binary: false,
        });
    }
    if let Some(s) = node.as_interpolated_string_node() {
        let mut value = Vec::new();
        for part in s.parts().iter() {
            if let Some(text) = string_value(&part, file) {
                value.extend(text.bytes);
            } else {
                // DstrNode#value joins decoded str children and the literal
                // interpolation source of other children, never evaluates it.
                let location = part
                    .as_embedded_variable_node()
                    .map_or_else(|| part.location(), |v| v.variable().location());
                let projected = file.match_pattern_source(location.as_slice());
                value.extend(projected.iter().enumerate().filter_map(|(i, b)| {
                    (*b != b'\r' || projected.get(i + 1) != Some(&b'\n')).then_some(*b)
                }));
            }
        }
        return Some(StringValue {
            bytes: value,
            binary: file.match_pattern_binary(),
        });
    }
    let s = node.as_string_node()?;
    let single = s.opening_loc().is_some_and(|l| {
        l.as_slice().starts_with(b"'")
            || l.as_slice().starts_with(b"%q")
            || l.as_slice().ends_with(b"'")
    });
    Some(StringValue {
        bytes: crate::cops::ast::require_literal::parts(
            s.content_loc().as_slice(),
            s.unescaped(),
            single,
            file,
        )
        .concat(),
        binary: file.match_pattern_binary(),
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/require_relative_self_path_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{
        config::ResolvedConfig,
        cops::{Registry, select_active},
        offense::Offense,
        pipeline,
    };
    use std::path::Path;
    fn offenses(s: &str, path: &str) -> Vec<Offense> {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(
            &registry,
            &config,
            &["Lint/RequireRelativeSelfPath".into()],
            &[],
        );
        let report = pipeline::lint_one(
            Path::new(path),
            Path::new(path),
            Some(s.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty(), "{s}: {report:?}");
        report.result.offenses
    }
    #[test]
    fn pinned_supplied_relative_and_absolute_paths() {
        for path in ["foo.rb", "/path/to/foo.rb"] {
            for arg in ["'foo'", "'foo.rb'", "\"f\\x6fo\"", "%q(foo)"] {
                let s = format!("require_relative {arg}\nrequire_relative 'bar'\n");
                let found = offenses(&s, path);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
                assert_eq!(
                    found[0].message,
                    "Remove the `require_relative` that requires itself."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn binary_literals_do_not_equal_non_ascii_utf8_filenames() {
        for encoding in ["ASCII-8BIT", "BINARY"] {
            for arg in [
                "'é'",
                "\"é.rb\"",
                "\"\\xC3\\xA9\"",
                "\"\\u00e9\"",
                "%Q'\\u00e9'",
                "\"\" \"é\"",
            ] {
                let source = format!("# coding: {encoding}\nrequire_relative {arg}\n");
                assert!(offenses(&source, "é.rb").is_empty(), "{source}");
            }
            let source = format!("# coding: {encoding}\nrequire_relative <<TEXT\né\nTEXT\n");
            assert!(offenses(&source, "é\n.rb").is_empty());
            let source = format!("# coding: {encoding}\nrequire_relative \"e\\u0301\"\n");
            assert!(offenses(&source, "e\u{301}.rb").is_empty());
            for (arg, path) in [("'foo'", "foo.rb"), ("__FILE__", "é.rb")] {
                let source = format!("# coding: {encoding}\nrequire_relative {arg}\n");
                let found = offenses(&source, path);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
            }
        }
        for arg in ["'é'", "\"\\xC3\\xA9\"", "\"\\u00e9\"", "\"\" \"é\""] {
            assert_eq!(
                offenses(&format!("require_relative {arg}\n"), "é.rb").len(),
                1
            );
        }
        assert_eq!(
            offenses("require_relative <<TEXT\né\nTEXT\n", "é\n.rb").len(),
            1
        );
    }
    #[test]
    fn real_canonical_whole_line_removal_in_closed_inactive_branch() {
        crate::cops::testing::assert_correction(
            &super::RequireRelativeSelfPath,
            "if false\n  require_relative 'test'\nend\n",
            "if false\nend\n",
        );
    }
    #[test]
    fn exact_argument_value_and_no_path_resolution() {
        for s in [
            "require_relative 'bar'",
            "require_relative",
            "require_relative 'foo.racc'",
            "Dir['test/**/test_*.rb'].each { |f| require_relative f }",
            "require_relative :foo",
            "require_relative './foo'",
            "require_relative '../foo'",
            "require_relative 'sub/foo'",
            "require_relative 1",
            "require_relative nil",
            "require_relative true",
            "require_relative __LINE__",
            "require_relative __ENCODING__",
            "require_relative `foo`",
            "obj&.require_relative 'foo'",
            "require_relative \"#{'foo'}\"",
            "require_relative *['foo']",
        ] {
            assert!(offenses(s, "foo.rb").is_empty(), "{s}");
        }
        for path in ["foo.rake", "Rakefile", "foo.RB", "foo.rb.bak"] {
            assert!(
                offenses("require_relative 'foo'", path).is_empty(),
                "{path}"
            );
        }
        for s in [
            "obj.require_relative 'foo'",
            "require_relative 'foo', extra",
            "require_relative 'foo', &block",
            "require_relative('foo') { nil }",
            "require_relative __FILE__",
        ] {
            assert_eq!(offenses(s, "foo.rb").len(), 1, "{s}");
        }
    }
}
