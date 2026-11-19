//! `Style/StringHashKeys` — opt-in, unsafe suggestions; never changes key identity.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StringHashKeys;
impl Cop for StringHashKeys {
    fn name(&self) -> &'static str {
        "Style/StringHashKeys"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_assoc_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(pair) = node.as_assoc_node() else {
            return;
        };
        let key = pair.key();
        let Some(string) = key.as_string_node() else {
            return;
        };
        if string
            .opening_loc()
            .is_some_and(|l| l.as_slice().starts_with(b"<<"))
        {
            return;
        }
        let valid = string.is_forced_binary_encoding()
            || if string.is_forced_utf8_encoding() {
                std::str::from_utf8(string.unescaped()).is_ok()
            } else {
                file.valid_source_encoded_bytes(string.unescaped())
            };
        if !valid || receives_strings(cx) {
            return;
        }
        let (line, col) = file.line_col(key.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer symbols instead of strings as hash keys.".into(),
            line,
            col,
            end_line: file.last_line_of(key.location()),
            // The key becomes its content's `to_sym.inspect`.
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: key.location().start_offset()..key.location().end_offset(),
                    replacement: String::from_utf8_lossy(
                        &crate::cops::lint::symbol_conversion::inspect_symbol(
                            string.unescaped(),
                            file,
                            false,
                        ),
                    )
                    .into_owned(),
                }),
        });
    }
}

fn receives_strings(cx: &NodeContext<'_, '_>) -> bool {
    // ArgumentsNode/StatementsNode are transparent; do not skip arrays, pairs,
    // parentheses or blocks. These exemptions are exact ancestor depths.
    let mut parents = cx
        .ancestors()
        .filter(|n| n.as_arguments_node().is_none() && n.as_statements_node().is_none());
    let _hash = parents.next();
    let Some(parent) = parents.next() else {
        return false;
    };
    let pipeline = parents
        .next()
        .and_then(|n| n.as_call_node())
        .is_some_and(|call| {
            call.call_operator_loc()
                .is_none_or(|l| l.as_slice() != b"&.")
                && call
                    .receiver()
                    .is_some_and(|n| crate::cops::ast::is_global_const(&n, &["Open3"]))
                && matches!(
                    call.name().as_slice(),
                    b"pipeline"
                        | b"pipeline_r"
                        | b"pipeline_rw"
                        | b"pipeline_start"
                        | b"pipeline_w"
                )
        });
    if pipeline {
        return true;
    }
    let Some(call) = parent.as_call_node() else {
        return false;
    };
    if call
        .call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
    {
        return false;
    }
    let name = call.name().as_slice();
    let recv = call.receiver();
    let global = |name| {
        recv.as_ref()
            .is_some_and(|n| crate::cops::ast::is_global_const(n, &[name]))
    };
    (global("IO") && name == b"popen")
        || (global("Open3")
            && matches!(
                name,
                b"capture2" | b"capture2e" | b"capture3" | b"popen2" | b"popen2e" | b"popen3"
            ))
        || ((recv.is_none() || global("Kernel")) && matches!(name, b"spawn" | b"system"))
        || matches!(name, b"gsub" | b"gsub!")
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop string_hash_keys_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str_with(&[&StringHashKeys], s)
    }
    #[test]
    fn pinned_offenses_and_unsafe_correction_calls() {
        for s in [
            "{ 'one' => 1 }",
            "{ 'one' => 1, two: 2, 3 => 3 }",
            "{ 'one two :' => 1 }",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(
                found[0].message,
                "Prefer symbols instead of strings as hash keys."
            );
            assert!(
                crate::cops::testing::correction_offenses(&StringHashKeys, s)[0]
                    .correction
                    .is_some()
            );
        }
    }
    #[test]
    fn decoded_static_keys_and_dynamic_symbol_lookalikes() {
        for s in [
            r#"{ "\x61" => 1 }"#,
            r#"{ "\u0061" => 1 }"#,
            "{ %q(a b) => 1 }",
            "{ ?a => 1 }",
            "{ \"a\nb\" => 1 }",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(found[0].end_line, if s.contains('\n') { 2 } else { 1 });
            assert!(found[0].correction.is_none());
        }
        for s in [
            r##"{ "#{key}" => 1 }"##,
            "{ 'a': 1 }",
            "{ :'a' => 1 }",
            "{ 'a' 'b' => 1 }",
            "{ name => 1 }",
            "{ **strings }",
            "IO = Object.new; IO.popen({'a'=>1})",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_no_offenses() {
        for s in [
            "{ one: 1 }",
            r#"{ "Test with malformed utf8 \251" => 'test' }"#,
            "IO.popen({\"RUBYOPT\" => '-w'}, 'ruby', 'foo.rb')",
            "Open3.capture3({\"RUBYOPT\" => '-w'}, 'ruby', 'foo.rb')",
            "Open3.pipeline([{\"RUBYOPT\" => '-w'}, 'ruby', 'foo.rb'], ['wc', '-l'])",
            r#""The sky is green.".gsub(/green/, "green" => "blue")"#,
            r#""The sky is green.".gsub!(/green/, "green" => "blue")"#,
            "{\n  <<~KEY => 1\n    text\n  KEY\n}\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn exact_environment_depths_namespaces_and_receiver_lookalikes() {
        for namespace in ["IO", "::IO", "Open3", "::Open3", "Kernel", "::Kernel", ""] {
            let methods: &[&str] = match namespace.trim_start_matches("::") {
                "IO" => &["popen"],
                "Open3" => &[
                    "capture2",
                    "capture2e",
                    "capture3",
                    "popen2",
                    "popen2e",
                    "popen3",
                ],
                _ => &["spawn", "system"],
            };
            let recv = if namespace.is_empty() {
                String::new()
            } else {
                format!("{namespace}.")
            };
            for method in methods {
                assert!(offenses(&format!("{recv}{method}({{'x' => 1}})")).is_empty());
                assert_eq!(
                    offenses(&format!("{recv}{method}({{a: {{'x' => 1}}}})")).len(),
                    1
                );
            }
        }
        for method in [
            "pipeline",
            "pipeline_r",
            "pipeline_rw",
            "pipeline_start",
            "pipeline_w",
        ] {
            assert!(offenses(&format!("::Open3.{method}([{{'x'=>1}}])")).is_empty());
            assert_eq!(offenses(&format!("Open3.{method}({{'x'=>1}})")).len(), 1);
            // Upstream's third-ancestor pattern does not require an Array.
            assert!(offenses(&format!("Open3.{method}(({{'x'=>1}}))")).is_empty());
        }
        for s in [
            "Other::IO.popen({'x'=>1})",
            "io.popen({'x'=>1})",
            "Other::Open3.capture3({'x'=>1})",
            "object.system({'x'=>1})",
            "IO&.popen({'x'=>1})",
            "s&.gsub(/x/, {'x'=>1})",
            "{'x'=>1}.gsub(/x/, {})",
            "{\"#{key}\"=>1}",
            "{x=>1}",
            "{'x':1}",
        ] {
            // The pinned exemption is lexical: it also covers a hash receiver.
            let expected = usize::from(
                ![
                    "{\"#{key}\"=>1}",
                    "{x=>1}",
                    "{'x':1}",
                    "{'x'=>1}.gsub(/x/, {})",
                ]
                .contains(&s),
            );
            assert_eq!(offenses(s).len(), expected, "{s}");
        }
    }
}
