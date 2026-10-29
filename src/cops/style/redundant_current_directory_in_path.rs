//! `Style/RedundantCurrentDirectoryInPath` — trim require_relative's ./ prefix.
use super::file_api::{argument, replacement};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantCurrentDirectoryInPath;
impl Cop for RedundantCurrentDirectoryInPath {
    fn name(&self) -> &'static str {
        "Style/RedundantCurrentDirectoryInPath"
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
        let Some(call) = node
            .as_call_node()
            .filter(|c| !c.is_safe_navigation() && c.name().as_slice() == b"require_relative")
        else {
            return;
        };
        let Some(first) = ast::call_arguments(&call).into_iter().next() else {
            return;
        };
        let segment = if first.as_string_node().is_some() {
            first
        } else if let Some(string) = first.as_interpolated_string_node() {
            let Some(part) = string.parts().iter().next() else {
                return;
            };
            part
        } else {
            return;
        };
        let Some(string) = segment.as_string_node() else {
            return;
        };
        let decoded = string.unescaped();
        let length = prefix_length(decoded);
        if length == 0 {
            return;
        }
        let content = string.content_loc();
        let raw = content.as_slice();
        let start = content.start_offset();
        let (line, col) = file.line_col(start);
        let end = start + length.min(raw.len());
        let correction = (cx.corrections_requested()
            && argument(&call).is_some()
            && call
                .receiver()
                .is_none_or(|r| ast::is_global_const(&r, &["Kernel"]))
            && raw.starts_with(&decoded[..length]))
        .then(|| {
            // One explicit pass removes every leading ./ component. Do not
            // expose an interpolation as the first path character: it could
            // produce an absolute path that was previously relative.
            let mut removed = length;
            while let n @ 1.. = prefix_length(&decoded[removed..]) {
                removed += n;
            }
            if removed == decoded.len() || !raw.starts_with(&decoded[..removed]) {
                return None;
            }
            let finish = ast::source_end(node);
            let before =
                std::str::from_utf8(&file.bytes[node.location().start_offset()..start]).ok()?;
            let after = std::str::from_utf8(&file.bytes[start + removed..finish]).ok()?;
            let mut edit = replacement(
                node,
                cx,
                file,
                format!("{before}{after}"),
                "RequireRelative",
            )?;
            // Validation covers the entire call; the actual edit only touches
            // the prefix and composes with quote/layout corrections.
            edit.range = start..start + removed;
            edit.replacement.clear();
            Some(edit)
        })
        .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Remove the redundant current directory path.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}
fn prefix_length(path: &[u8]) -> usize {
    if !path.starts_with(b"./") {
        return 0;
    }
    1 + path[1..].iter().take_while(|&&b| b == b'/').count()
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_current_directory_in_path_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn custom_receivers_bindings_and_dynamic_path_boundaries_withhold_edits() {
        for source in [
            "loader.require_relative './feature'",
            "self.require_relative './feature'",
            "require_relative './feature', extra()",
            "require_relative './feature', &block",
            "require_relative('./feature') { effect }",
            "require_relative \"./#{path}\"",
            "require_relative \".///./#{path}\"",
            "require_relative './'",
            "require_relative \"\\x2e/feature\"",
            "require_relative <<~PATH\n ./feature\nPATH\n",
            "require_relative( # retain\n './feature')",
        ] {
            let found = correction_offenses(&RedundantCurrentDirectoryInPath, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for binding in [
            "def require_relative(path); path; end",
            "alias require_relative puts",
            "define_method(:require_relative) { |p| p }",
            "attr_reader :require_relative",
            "Kernel.define_singleton_method(:require_relative) { |p| p }",
            "Kernel.module_eval { alias require_relative puts }",
            "Kernel.module_eval(dynamic)",
            "Kernel = Custom",
            "Object.send(:const_set, :Kernel, Custom)",
        ] {
            let source = format!("{binding}\nrequire_relative './feature'\n");
            let found = correction_offenses(&RedundantCurrentDirectoryInPath, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn preserves_spelling_headers_crlf_and_literal_interpolation() {
        for receiver in ["", "Kernel.", "::Kernel."] {
            for (literal, fixed) in [
                ("%Q(./dir/#{name})", "%Q(dir/#{name})"),
                ("%q{./dir/\\'file}", "%q{dir/\\'file}"),
                ("\"./dir\\x2ffeature\"", "\"dir\\x2ffeature\""),
            ] {
                assert_correction(
                    &RedundantCurrentDirectoryInPath,
                    &format!("{receiver}require_relative {literal}\n"),
                    &format!("{receiver}require_relative {fixed}\n"),
                );
            }
        }
        assert_correction(
            &RedundantCurrentDirectoryInPath,
            "\u{feff}# encoding: UTF-8\r\nrequire_relative './feature'\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nrequire_relative 'feature'\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }
    #[test]
    fn pinned_offenses_and_single_pass_corrections() {
        for (before, after, col) in [
            ("'./path/to/feature'", "'path/to/feature'", 19),
            ("'./path/#{to}/feature'", "'path/#{to}/feature'", 19),
            ("\"./path/#{to}/feature\"", "\"path/#{to}/feature\"", 19),
            ("'.///./../path/to/feature'", "'../path/to/feature'", 19),
            ("%q(./path/to/feature)", "%q(path/to/feature)", 21),
        ] {
            let source = format!("# header\n  require_relative {before}\n");
            let found = correction_offenses(&RedundantCurrentDirectoryInPath, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col + 2, 2)
            );
            assert_eq!(
                found[0].message,
                "Remove the redundant current directory path."
            );
            assert_correction(
                &RedundantCurrentDirectoryInPath,
                &source,
                &format!("# header\n  require_relative {after}\n"),
            );
        }
    }
    #[test]
    fn pinned_no_offense_and_lookalikes() {
        for source in [
            "require_relative '../path/to/feature'",
            "require_relative '.path'",
            "require_relative 'p/t/feature'",
            "require_relative 'path/to/feature'",
            "require_relative \"path/#{to}/feature\"",
            "do_something './path/to/feature'",
            "do_something",
            "require_relative",
            "require_relative path",
            "require_relative :path",
            "require_relative \"#{prefix}./feature\"",
            "loader&.require_relative './feature'",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/RedundantCurrentDirectoryInPath"),
                "{source}"
            );
        }
    }
}
