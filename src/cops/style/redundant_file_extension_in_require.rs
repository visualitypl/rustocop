//! `Style/RedundantFileExtensionInRequire` — omit Ruby feature extensions.
use super::file_api::{argument, replacement};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantFileExtensionInRequire;
impl Cop for RedundantFileExtensionInRequire {
    fn name(&self) -> &'static str {
        "Style/RedundantFileExtensionInRequire"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node().filter(|c| {
            !c.is_safe_navigation()
                && c.receiver().is_none()
                && matches!(c.name().as_slice(), b"require" | b"require_relative")
        }) else {
            return;
        };
        let Some(string) = argument(&call).and_then(|n| n.as_string_node()) else {
            return;
        };
        if !string.unescaped().ends_with(b".rb") {
            return;
        }
        let content = string.content_loc();
        let raw = content.as_slice();
        // Keep RuboCop's escaped-extension exemption. For escaped suffixes or
        // heredocs retain detection, but never remove arbitrary source bytes.
        let literal_suffix = raw.ends_with(b".rb");
        if raw.get(raw.len().wrapping_sub(4)) == Some(&b'\\') {
            return;
        }
        let end = content.end_offset();
        let start = end.saturating_sub(3).max(content.start_offset());
        // Removing .rb must not turn an explicit Ruby filename into another
        // explicit Ruby/native filename. Inspect decoded bytes, including
        // escaped stems, and conservatively include native suffixes on other
        // supported Ruby platforms without probing the filesystem.
        let stem = &string.unescaped()[..string.unescaped().len() - 3];
        let has_extension = [
            b".rb".as_slice(),
            b".so",
            b".o",
            b".bundle",
            b".dll",
            b".dylib",
        ]
        .iter()
        .any(|suffix| stem.ends_with(suffix));
        let correction =
            (cx.corrections_requested() && literal_suffix && !stem.is_empty() && !has_extension)
                .then(|| {
                    let call_start = node.location().start_offset();
                    let call_end = ast::source_end(node);
                    let before = std::str::from_utf8(&file.bytes[call_start..start]).ok()?;
                    let after = std::str::from_utf8(&file.bytes[end..call_end]).ok()?;
                    let api = if call.name().as_slice() == b"require" {
                        "Require"
                    } else {
                        "RequireRelative"
                    };
                    let mut edit = replacement(node, cx, file, format!("{before}{after}"), api)?;
                    edit.range = start..end;
                    edit.replacement.clear();
                    Some(edit)
                })
                .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Redundant `.rb` file extension detected.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_file_extension_in_require_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == RedundantFileExtensionInRequire.name())
            .collect()
    }
    #[test]
    fn pinned_offenses_and_corrections() {
        let source = "require 'foo.rb'\nrequire_relative '../foo.rb'\n";
        let found = offenses(source);
        assert_eq!(found.len(), 2);
        for (o, position) in found.iter().zip([(1, 13), (2, 25)]) {
            assert_eq!((o.line, o.col), position);
            assert_eq!(o.message, "Redundant `.rb` file extension detected.");
        }
        assert_correction(
            &RedundantFileExtensionInRequire,
            source,
            "require 'foo'\nrequire_relative '../foo'\n",
        );
    }
    #[test]
    fn literal_spellings_headers_crlf_and_source_data_are_preserved() {
        for (before, after) in [
            ("'caf\u{e9}.rb'", "'caf\u{e9}'"),
            ("\"foo.rb\"", "\"foo\""),
            ("%q{dir/feature.rb}", "%q{dir/feature}"),
            ("%Q(dir/feature.rb)", "%Q(dir/feature)"),
            (r#""dir\x2ffeature.rb""#, r#""dir\x2ffeature""#),
            (r#""\u0066oo.rb""#, r#""\u0066oo""#),
            (r#""\061.rb""#, r#""\061""#),
            ("\"dir/\\\nfeature.rb\"", "\"dir/\\\nfeature\""),
        ] {
            for method in ["require", "require_relative"] {
                assert_correction(
                    &RedundantFileExtensionInRequire,
                    &format!("{method} {before}\n"),
                    &format!("{method} {after}\n"),
                );
            }
        }
        assert_correction(
            &RedundantFileExtensionInRequire,
            "\u{feff}# encoding: UTF-8\r\nrequire_relative 'feature.rb'\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nrequire_relative 'feature'\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }
    #[test]
    fn custom_binders_refinements_and_source_sensitive_paths_withhold_edits() {
        for method in ["require", "require_relative"] {
            for binder in [
                format!("def {method}(path); path; end"),
                format!("alias {method} puts"),
                format!("define_method(:{method}) {{ |p| p }}"),
                format!("attr_reader :{method}"),
                format!("Kernel.define_singleton_method(:{method}) {{ |p| p }}"),
                format!("Kernel.module_eval {{ alias {method} puts }}"),
                format!("Kernel.singleton_class.class_eval {{ attr_reader :{method} }}"),
                format!("Kernel.send(:define_method, :{method}) {{ |p| p }}"),
                format!("undef {method}"),
                format!("alias_method :{method}, :puts"),
                format!("Class.new(Struct.new(:{method}))"),
                "Kernel.module_eval(dynamic)".into(),
                "Kernel = Custom".into(),
                "Object.send(:const_set, :Kernel, Custom)".into(),
                "module Paths; refine Kernel do; end; end".into(),
            ] {
                let source = format!("{binder}\n{method} 'feature.rb'\n");
                let found = correction_offenses(&RedundantFileExtensionInRequire, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
        for source in [
            r#"require "foo\x2erb""#,
            r#"require_relative "foo.r\u0062""#,
            "require '.rb'",
            "require_relative('foo.rb') { effect }",
            "require( # comment\n 'foo.rb')",
        ] {
            let found = correction_offenses(&RedundantFileExtensionInRequire, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        // Unrelated binders do not suppress eligible canonical fixes.
        assert_correction(
            &RedundantFileExtensionInRequire,
            "def other; end\nrequire 'foo.rb'",
            "def other; end\nrequire 'foo'",
        );
    }
    #[test]
    fn recognized_extensions_in_decoded_stems_withhold_edits() {
        for stem in [
            "feature.rb",
            "feature.so",
            "feature.o",
            "feature.bundle",
            "feature.dll",
            "feature.dylib",
            r"feature\x2erb",
        ] {
            for method in ["require", "require_relative"] {
                let source = format!("{method} \"{stem}.rb\"");
                let found = correction_offenses(&RedundantFileExtensionInRequire, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn escaped_suffix_exemption_matches_reference_source_range() {
        // The reference's final-three-source-byte range can itself follow
        // a backslash even when the decoded extension has no backslash.
        assert!(offenses(r#"require "foo.r\x62""#).is_empty());
        let found = offenses(r#"require_relative "foo.r\u0062""#);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 27));
    }
    #[test]
    fn pinned_negatives_and_lookalikes() {
        for source in [
            r"require 'foo\.rb'",
            r"require 'foo\\.rb'",
            "require 'foo.so'",
            "require_relative '../foo.so'",
            "require 'foo'",
            "require_relative '../foo'",
            "require name",
            "require_relative name",
            "Kernel.require 'foo.rb'",
            "::Kernel.require_relative 'foo.rb'",
            "self.require 'foo.rb'",
            "loader&.require 'foo.rb'",
            "require :foo",
            "require 'foo.rb', extra",
            "require 'foo.rb', &block",
            "require",
            "require *['foo.rb']",
            "require \"#{name}.rb\"",
            "require_relative(path: 'foo.rb')",
            "load 'foo.rb'",
            "require = 'foo.rb'; require",
            "require 'foo.RB'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
