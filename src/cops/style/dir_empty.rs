//! `Style/DirEmpty` — direct directory emptiness checks (Ruby >= 2.4).
use super::file_api::{argument, integer, no_arguments, replacement, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DirEmpty;
impl Cop for DirEmpty {
    fn name(&self) -> &'static str {
        "Style/DirEmpty"
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
        let Some((constant, arg, negative)) = offensive(node) else {
            return;
        };
        let constant = String::from_utf8_lossy(constant.location().as_slice());
        let arg_text = String::from_utf8_lossy(arg.location().as_slice());
        let text = format!(
            "{}{constant}.empty?({arg_text})",
            if negative { "!" } else { "" }
        );
        let (line, col) = file.line_col(node.location().start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Unlike enumeration, Dir.empty? returns false for regular
                // files and missing paths instead of raising ENOTDIR/ENOENT.
                // Only intrinsic directory literals have a proven precondition
                // without inspecting mutable filesystem state or re-evaluating
                // a user path. Assume no concurrent filesystem mutation, as for
                // the original emptiness check itself.
                let directory = arg
                    .as_string_node()
                    .is_some_and(|s| matches!(s.unescaped(), b"." | b"./" | b"/"));
                if !directory || std::str::from_utf8(arg.location().as_slice()).is_err() {
                    return None;
                }
                // `!` binds less tightly than calls and comparison operators.
                let text = if negative && cx.parent().is_some_and(|p| p.as_call_node().is_some()) {
                    format!("({text})")
                } else {
                    text.clone()
                };
                replacement(node, cx, file, text, "Dir")
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{text}` instead."),
            line,
            col,
            end_line: file.line_of(
                ast::source_end(node)
                    .saturating_sub(1)
                    .max(node.location().start_offset()),
            ),
            correction,
        });
    }
}

fn offensive<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, bool)> {
    let call = send(node)?;
    let name = call.name().as_slice();
    let receiver = call.receiver()?;
    let (directory, method, expected) = match name {
        b"==" | b"!=" | b">" => {
            let size = send(&receiver)?;
            if size.name().as_slice() != b"size" || !no_arguments(&size) {
                return None;
            }
            (size.receiver()?, None, Some(argument(&call)?))
        }
        b"empty?" if no_arguments(&call) => (receiver, Some(b"children".as_slice()), None),
        b"none?" if no_arguments(&call) => (receiver, Some(b"each_child".as_slice()), None),
        _ => return None,
    };
    let directory = send(&directory)?;
    let dir_method = directory.name().as_slice();
    if let Some(expected) = expected {
        let value = match dir_method {
            b"entries" => 2,
            b"children" => 0,
            _ => return None,
        };
        if !integer(&expected, value) {
            return None;
        }
    } else if Some(dir_method) != method {
        return None;
    }
    let constant = directory.receiver()?;
    if !ast::is_global_const(&constant, &["Dir"]) {
        return None;
    }
    Some((
        constant,
        argument(&directory)?,
        matches!(name, b"!=" | b">"),
    ))
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/dir_empty_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn pinned_diagnostics_and_corrections() {
        for constant in ["Dir", "::Dir"] {
            for (tail, bang) in [
                ("entries('path/to/dir').size == 2", ""),
                ("entries('path/to/dir').size != 2", "!"),
                ("entries('path/to/dir').size > 2", "!"),
                ("children('path/to/dir').size == 0", ""),
                ("children('path/to/dir').size != 0", "!"),
                ("children('path/to/dir').size > 0", "!"),
                ("children('path/to/dir').empty?", ""),
                ("each_child('path/to/dir').none?", ""),
            ] {
                let source = format!("# header\n  {constant}.{tail}\n");
                let replacement = format!("{bang}{constant}.empty?('path/to/dir')");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/DirEmpty")
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(found[0].message, format!("Use `{replacement}` instead."));
                // Pinned detection remains exact, but this arbitrary path may
                // be a file or missing, so the upstream edit is withheld.
                assert_correction(&DirEmpty, &source, &source);
                for path in ["'.'", "'./'", "'/'", "\"\\x2e\""] {
                    assert_correction(
                        &DirEmpty,
                        &source.replace("'path/to/dir'", path),
                        &format!("# header\n  {replacement}\n").replace("'path/to/dir'", path),
                    );
                }
            }
        }
        assert_correction(
            &DirEmpty,
            "Dir.\n  entries('.').size == 2\n",
            "Dir.empty?('.')\n",
        );
        assert_correction(
            &DirEmpty,
            "!Dir.each_child('.').none?\n",
            "!Dir.empty?('.')\n",
        );
    }

    #[test]
    fn withholds_paths_that_may_be_missing_or_regular_files() {
        for arg in ["path", "next_path()", "'regular'", "'missing'", "__dir__"] {
            for tail in [
                "entries({}).size == 2",
                "children({}).empty?",
                "each_child({}).none?",
            ] {
                let source = format!("Dir.{}", tail.replace("{}", arg));
                let found = correction_offenses(&DirEmpty, &source);
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }

    #[test]
    fn withholds_unproven_paths_and_preserves_negation_binding_for_directory_literals() {
        for arg in [
            "path",
            "next_path()",
            "\"a\\x2fb\"",
            "%q(a/b)",
            "\"#{path}/child\"",
            "(path = next_path)",
            "helper { next_path }",
            "\"a\nb\"",
            "\"# not a comment\"",
        ] {
            assert_correction(
                &DirEmpty,
                &format!("::Dir.children({arg}).empty?\n"),
                &format!("::Dir.children({arg}).empty?\n"),
            );
        }
        assert_correction(
            &DirEmpty,
            "Dir.children('.').size.>(0).to_s\n",
            "(!Dir.empty?('.')).to_s\n",
        );
        assert_correction(
            &DirEmpty,
            "Dir.children('.').size.>(0) == expected\n",
            "(!Dir.empty?('.')) == expected\n",
        );
        assert_correction(
            &DirEmpty,
            "consume(Dir.children('.').size != 0)\n",
            "consume(!Dir.empty?('.'))\n",
        );
        assert_correction(
            &DirEmpty,
            "\u{feff}# encoding: UTF-8\r\nx = Dir.children('.').empty? # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nx = Dir.empty?('.') # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }

    #[test]
    fn literal_integer_values_not_spellings_control_matches() {
        for source in [
            "Dir.entries(path).size == 0x2",
            "Dir.children(path).size == 0b0",
            "Dir.children(path).size == +0",
            "Dir.children(path).size == -0",
        ] {
            assert_eq!(correction_offenses(&DirEmpty, source).len(), 1, "{source}");
        }
    }

    #[test]
    fn rejects_nonmatching_chains_and_blocks() {
        for source in [
            "Dir.empty?(path)",
            "Dir.exist?(path)",
            "Other::Dir.children(path).empty?",
            "dir.children(path).empty?",
            "Dir.entries(path).size == 0",
            "Dir.children(path).size == 2",
            "Dir.children(path).length == 0",
            "Dir.children(path) == 0",
            "Dir.entries(path).size >= 2",
            "0 == Dir.children(path).size",
            "Dir.children(path, encoding: 'UTF-8').empty?",
            "Dir.children.empty?",
            "Dir.children(path).empty?(arg)",
            "Dir.children(path, &b).empty?",
            "Dir.children(path).empty?(&b)",
            "Dir.children(path) { work }.empty?",
            "Dir.each_child(path).none? { |f| f.start_with?('.') }",
            "Dir.each_child(path).none? do |f| work(f) end",
            "Dir.each_child(path) { work }.none?",
            "Dir&.children(path).empty?",
            "Dir.children(path)&.empty?",
            "Dir.children(path).size(&b) == 0",
        ] {
            assert!(
                correction_offenses(&DirEmpty, source).is_empty(),
                "{source}"
            );
        }
    }
}
