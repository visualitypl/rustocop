//! `Lint/NumberConversion` — opt-in, detection-only strict numeric parsing advice.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct NumberConversion;
impl Cop for NumberConversion {
    fn name(&self) -> &'static str {
        "Lint/NumberConversion"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some((receiver, method, end, safe)) = getter(node) {
            if constructor(&method).is_some() && !allowed_receiver(&receiver, file) {
                let text = source(&receiver, file);
                let op = if safe { "&." } else { "." };
                let method_text = std::str::from_utf8(&method).unwrap();
                let message = advice(
                    &format!("{text}{op}{method_text}"),
                    &replacement(&method, &text),
                );
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    node.location().start_offset(),
                    file,
                );
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message,
                    line,
                    col,
                    end_line: file.line_of(end.saturating_sub(1)),
                    correction: None,
                });
            }
            return;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        let receiver = call.receiver();
        let args = ast::call_arguments(&call);
        let message = if args.is_empty()
            && constructor(call.name().as_slice()).is_some()
            && receiver
                .as_ref()
                .is_some_and(|r| !allowed_receiver(r, file))
        {
            let source = source(receiver.as_ref().unwrap(), file);
            let op = if call.is_safe_navigation() { "&." } else { "." };
            let method = std::str::from_utf8(call.name().as_slice()).unwrap();
            Some(advice(
                &format!("{source}{op}{method}"),
                &replacement(call.name().as_slice(), &source),
            ))
        } else if args.len() == 1 && !operator(call.name().as_slice()) {
            let arg = &args[0];
            let expression = arg.as_block_argument_node().and_then(|b| b.expression());
            expression
                .as_ref()
                .unwrap_or(arg)
                .as_symbol_node()
                .and_then(|s| {
                    let name = s.unescaped();
                    constructor(name)?;
                    Some(advice(
                        &source(arg, file),
                        &format!("{{ |i| {} }}", replacement(name, "i")),
                    ))
                })
        } else {
            None
        };
        let Some(message) = message else { return };
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
// Prism folds the getter send into compound call writes. Its source stops
// at the selector, not at the assignment value. Multi-target setters are not
// getters, and index operators are excluded by both pinned matchers.
fn getter<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Vec<u8>, usize, bool)> {
    macro_rules! get {
        ($access:ident) => {{
            let n = node.$access()?;
            (
                n.receiver()?,
                n.read_name().as_slice().to_vec(),
                n.message_loc()?.end_offset(),
                n.is_safe_navigation(),
            )
        }};
    }
    Some(match node {
        Node::CallAndWriteNode { .. } => get!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => get!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => get!(as_call_operator_write_node),
        _ => return None,
    })
}
fn constructor(method: &[u8]) -> Option<&'static str> {
    Some(match method {
        b"to_i" => "Integer",
        b"to_f" => "Float",
        b"to_c" => "Complex",
        b"to_r" => "Rational",
        _ => return None,
    })
}
fn replacement(method: &[u8], object: &str) -> String {
    let tail = if method == b"to_i" { ", 10" } else { "" };
    format!("{}({object}{tail})", constructor(method).unwrap())
}
fn advice(current: &str, corrected: &str) -> String {
    format!(
        "Replace unsafe number conversion with number class parsing, instead of using `{current}`, use stricter `{corrected}`."
    )
}
fn source(node: &Node<'_>, file: &SourceFile) -> String {
    let start = node.location().start_offset();
    let bytes = file.match_pattern_source(&file.bytes[start..node.location().end_offset()]);
    // Binary bytes cannot appear literally in the UTF-8 diagnostic protocol.
    bytes
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect::<String>()
        .replace("\r\n", " ")
        .replace('\n', " ")
}
fn operator(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
            | b"!@"
            | b"~@"
            | b"`"
    )
}
fn allowed_receiver(node: &Node<'_>, file: &SourceFile) -> bool {
    if matches!(
        node,
        Node::IntegerNode { .. }
            | Node::SourceLineNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
    ) || ast::negated_numeric_literal(node).is_some()
    {
        return true;
    }
    // A literal block is a block node upstream, not a call for this exemption.
    if let Some(call) = node
        .as_call_node()
        .filter(|c| !c.block().is_some_and(|b| b.as_block_node().is_some()))
        && matches!(
            call.name().as_slice(),
            b"Integer"
                | b"Float"
                | b"Complex"
                | b"Rational"
                | b"to_i"
                | b"to_f"
                | b"to_c"
                | b"to_r"
        )
    {
        return true;
    }
    let mut cache = file.number_conversion_roots.lock().unwrap();
    let mut current = None;
    let mut path = Vec::new();
    let allowed = loop {
        let node = current.as_ref().unwrap_or(node);
        let key = (node.location().start_offset(), node.location().end_offset());
        if let Some(allowed) = cache.get(&key) {
            break *allowed;
        }
        path.push(key);
        if let Some(receiver) = node.as_call_node().and_then(|c| c.receiver()) {
            current = Some(receiver);
        } else {
            break ast::is_global_const(node, &["Time"])
                || ast::is_global_const(node, &["DateTime"]);
        }
    };
    for key in path {
        cache.insert(key, allowed);
    }
    allowed
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/number_conversion_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{
        config::ResolvedConfig,
        cops::{Registry, select_active},
        offense::Offense,
        pipeline::lint_one,
    };
    use std::path::Path;
    fn offenses(source: &str) -> Vec<Offense> {
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Lint/NumberConversion".into()], &[]);
        let report = lint_one(
            Path::new("example.rb"),
            Path::new("example.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty(), "{source}: {report:?}");
        report.result.offenses
    }
    #[test]
    fn direct_conversions_exact_message_position_and_no_edits() {
        for (method, class, tail) in [
            ("to_i", "Integer", ", 10"),
            ("to_f", "Float", ""),
            ("to_c", "Complex", ""),
            ("to_r", "Rational", ""),
        ] {
            for receiver in ["\"10\"", "value", "params[:id]", "args[0]", "foo&.bar"] {
                for op in [".", "&."] {
                    let source = format!("# numeric\n  {receiver}{op}{method}\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        format!(
                            "Replace unsafe number conversion with number class parsing, instead of using `{receiver}{op}{method}`, use stricter `{class}({receiver}{tail})`."
                        )
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn symbols_and_block_pass_are_distinct_from_operators_or_multiple_arguments() {
        for (source, current, corrected) in [
            ("values.map(&:to_i)", "&:to_i", "{ |i| Integer(i, 10) }"),
            ("map(&:to_i)", "&:to_i", "{ |i| Integer(i, 10) }"),
            ("try(:to_f)", ":to_f", "{ |i| Float(i) }"),
            ("values&.map &:to_r", "&:to_r", "{ |i| Rational(i) }"),
            ("\"foo\".try(:to_f)", ":to_f", "{ |i| Float(i) }"),
            ("\"foo\".send(:to_c)", ":to_c", "{ |i| Complex(i) }"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Replace unsafe number conversion with number class parsing, instead of using `{current}`, use stricter `{corrected}`."
                )
            );
        }
        for source in [
            "delegate :to_f, to: :description",
            "method_name == :to_i",
            "hash[:to_f]",
            "foo.try(:to_i, other)",
            "foo.map(1, &:to_i)",
            "foo.to_i(10)",
            "foo.to_i(&blk)",
            "foo.try(:to_i, &blk)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn blocks_are_receiver_source_and_messages_stay_on_one_protocol_line() {
        let found = offenses("foo {}.to_i\n");
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("`Integer(foo {}, 10)`"));
        let found = offenses("case foo.to_f\nwhen 0.0\nbar\nelse\nbaz\nend.to_i\n");
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|o| !o.message.contains('\n')));
        let outer = found.iter().find(|o| o.col == 1).unwrap();
        assert_eq!((outer.line, outer.end_line), (1, 6));
        assert!(
            outer
                .message
                .contains("Integer(case foo.to_f when 0.0 bar else baz end, 10)")
        );
    }
    #[test]
    fn receiver_root_cache_is_linear_for_repeated_conversion_chains() {
        use crate::{
            cops::walk::{NodeVisitor, walk},
            source::SourceFile,
        };
        struct Check<'a> {
            file: &'a SourceFile,
            checked: usize,
        }
        impl<'pr> NodeVisitor<'pr> for Check<'_> {
            fn enter(&mut self, node: ruby_prism::Node<'pr>) {
                if let Some(c) = node.as_call_node()
                    && c.name().as_slice() == b"to_i"
                {
                    assert!(!super::allowed_receiver(&c.receiver().unwrap(), self.file));
                    self.checked += 1;
                }
            }
        }
        let text = format!("value{}", ".other.to_i".repeat(20));
        let parsed = ruby_prism::parse(text.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let file = SourceFile::new("example.rb".into(), text.as_bytes().to_vec());
        let mut check = Check {
            file: &file,
            checked: 0,
        };
        walk(&mut check, &parsed.node());
        assert_eq!(check.checked, 20);
        assert_eq!(file.number_conversion_roots.lock().unwrap().len(), 40);
    }
    #[test]
    fn compound_assignment_getters_are_real_sends_but_targets_are_setters() {
        for source in [
            "value.to_i ||= other",
            "value&.to_f &&= other",
            "value.to_r += other",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
        for source in [
            "value.to_i, other = values",
            "value.to_i = other",
            "value.to_i(1)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn numeric_and_constructor_receivers_and_fixed_allowed_classes() {
        for source in [
            "42.to_i",
            "__LINE__.to_i",
            "42.0.to_f",
            "1r.to_c",
            "1i.to_r",
            "-42.to_i",
            "Integer(var, 10).to_f",
            "Float(var).to_i",
            "Complex(var).to_f",
            "Rational(var).to_c",
            "Time.now.to_i",
            "DateTime.new(2012,8,29).to_f",
            "Time.zone.now.to_datetime.to_f",
            "::Time.now.to_i",
            "Time.now.to_i.foo.to_i",
            "Time.new {}.to_i",
            "to_i",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "(42).to_i",
            "10.minutes.to_i",
            "10.hours.to_i",
            "Other::Time.now.to_i",
            "Time::Zone.now.to_i",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        let found = offenses("var.to_i.to_f");
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("`var.to_i`"));
    }
}
