//! `Style/YAMLFileRead` — YAML convenience file readers.
use super::file_api::{argument, send, yaml_replacement};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct YAMLFileRead;
impl Cop for YAMLFileRead {
    fn name(&self) -> &'static str {
        "Style/YAMLFileRead"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || !matches!(call.name().as_slice(), b"load" | b"safe_load" | b"parse")
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["YAML"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let Some((first, rest)) = args.split_first() else {
            return;
        };
        let Some(read) = send(first) else { return };
        if read.name().as_slice() != b"read"
            || !read
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["File"]))
        {
            return;
        }
        let Some(path) = argument(&read) else { return };
        let method = String::from_utf8_lossy(call.name().as_slice());
        let rest_source = if rest.is_empty() {
            String::new()
        } else {
            format!(
                ", {}",
                rest.iter()
                    .map(|n| String::from_utf8_lossy(n.location().as_slice()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let prefer = format!(
            "{method}_file({}{rest_source})",
            String::from_utf8_lossy(path.location().as_slice())
        );
        // File helpers set filename metadata and open in bom|utf-8, unlike
        // File.read. Without known contents even explicit filename:nil cannot
        // prove equivalence (UTF-16 BOM errors differ). Only the null device is
        // provably empty without reading application files or evaluating Ruby.
        let correction =
            (cx.corrections_requested() && null_device(&path) && safe_options(rest, &method))
                .then(|| {
                    let receiver = call.receiver()?;
                    let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
                    // Never write replacement characters for legacy source bytes.
                    std::str::from_utf8(node.location().as_slice()).ok()?;
                    yaml_replacement(node, cx, file, format!("{receiver}.{prefer}"))
                })
                .flatten();
        let start = call.message_loc().unwrap_or(call.location()).start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{prefer}` instead."),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction,
        });
    }
}

fn null_device(path: &Node<'_>) -> bool {
    path.as_constant_path_node().is_some_and(|p| {
        p.name().is_some_and(|n| n.as_slice() == b"NULL")
            && p.parent()
                .is_some_and(|p| ast::is_global_const(&p, &["File"]))
    })
}

fn safe_options(rest: &[Node<'_>], method: &str) -> bool {
    if rest.is_empty() {
        return true;
    }
    if method == "parse" {
        return false;
    }
    let [options] = rest else { return false };
    let Some(hash) = options.as_keyword_hash_node() else {
        return false;
    };
    hash.elements().iter().all(|n| {
        let Some(pair) = n.as_assoc_node() else {
            return false;
        };
        let Some(key) = pair.key().as_symbol_node() else {
            return false;
        };
        // Unknown keywords can change ArgumentError backtraces/signatures.
        if !matches!(
            key.unescaped(),
            b"filename"
                | b"fallback"
                | b"aliases"
                | b"symbolize_names"
                | b"freeze"
                | b"strict_integer"
        ) {
            return false;
        }
        let value = pair.value();
        value.as_nil_node().is_some()
            || value.as_false_node().is_some()
            || value.as_true_node().is_some()
            || value.as_string_node().is_some()
            || value.as_integer_node().is_some()
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/yaml_file_read_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_methods_paths_and_keywords_report_at_the_selector() {
        for yaml in ["YAML", "::YAML"] {
            for file in ["File", "::File"] {
                for method in ["load", "safe_load", "parse"] {
                    for path in [
                        "path",
                        "'file.yml'",
                        "%q(file.yml)",
                        "\"fi\\x6ce.yml\"",
                        "Rails.root.join('foo', 'bar', 'baz')",
                        "File::NULL",
                    ] {
                        for rest in ["", ", permitted_classes: [Regexp, Symbol], aliases: true"] {
                            let source =
                                format!("# header\n  {yaml}.{method}({file}.read({path}){rest})\n");
                            let found: Vec<_> = lint_str(&source)
                                .into_iter()
                                .filter(|o| o.cop == "Style/YAMLFileRead")
                                .collect();
                            assert_eq!(found.len(), 1, "{source}");
                            assert_eq!((found[0].line, found[0].col), (2, yaml.len() as u32 + 4));
                            assert_eq!(
                                found[0].message,
                                format!("Use `{method}_file({path}{rest})` instead.")
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn receiver_argument_and_block_lookalikes() {
        for source in [
            "YAML.load(yaml)",
            "YAML.load_file(File.read(path))",
            "Psych.load(File.read(path))",
            "Other::YAML.load(File.read(path))",
            "yaml.load(File.read(path))",
            "YAML&.load(File.read(path))",
            "YAML.load(Other::File.read(path))",
            "YAML.load(file.read(path))",
            "YAML.load(File&.read(path))",
            "YAML.load(File.read)",
            "YAML.load(File.read(path, 100))",
            "YAML.load(File.read(path, encoding: 'UTF-8'))",
            "YAML.load(File.read(path) {})",
            "YAML.unsafe_load(File.read(path))",
        ] {
            assert!(
                correction_offenses(&YAMLFileRead, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn null_device_is_a_provably_empty_canonical_safe_case() {
        for receiver in ["YAML", "::YAML"] {
            for method in ["load", "safe_load", "parse"] {
                for path in ["File::NULL", "::File::NULL"] {
                    let source = format!("{receiver}.{method}(::File.read({path}))\n");
                    assert_correction(
                        &YAMLFileRead,
                        &source,
                        &format!("{receiver}.{method}_file({path})\n"),
                    );
                }
            }
        }
        assert_correction(
            &YAMLFileRead,
            "\u{feff}# encoding: UTF-8\r\nx = YAML.safe_load(File.read(File::NULL), fallback: 'empty', aliases: true) # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nx = YAML.safe_load_file(File::NULL, fallback: 'empty', aliases: true) # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }
    #[test]
    fn uncertain_file_contents_filename_metadata_and_keyword_evaluation_are_not_rewritten() {
        for source in [
            "YAML.load(File.read(path))",
            "YAML.safe_load(File.read('file.yml'))",
            "YAML.parse(File.read('file.yml'))",
            "YAML.load(File.read('file.yml'), filename: nil)",
            "YAML.safe_load(File.read('file.yml'), permitted_classes: [Regexp, Symbol], aliases: true)",
            "YAML.load(File.read(File::NULL), fallback: compute)",
            "YAML.load(File.read(File::NULL), **options)",
            "YAML.load(File.read(File::NULL), *options)",
            "YAML.load(File.read(File::NULL), positional)",
            "YAML.load(File.read(File::NULL), &block)",
            "YAML.load(File.read(File::NULL)) { side_effect }",
            "YAML.parse(File.read(File::NULL), fallback: 'changed')",
            "YAML.parse(File.read(File::NULL), filename: nil)",
            "YAML.load(File.read(File::NULL), unknown: true)",
            "YAML.load( # keep\n File.read(File::NULL))",
            "YAML.load(\n File.read(File::NULL))\np __LINE__\n",
        ] {
            let found = correction_offenses(&YAMLFileRead, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
