//! `Lint/SuppressedExceptionInNumberConversion` — swallowed constructor errors.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SuppressedExceptionInNumberConversion;
impl Cop for SuppressedExceptionInNumberConversion {
    fn name(&self) -> &'static str {
        "Lint/SuppressedExceptionInNumberConversion"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((expression, start, end)) = rescued_expression(node, cx) else {
            return;
        };
        let Some(call) = expression.as_call_node() else {
            return;
        };
        let args = ast::call_arguments(&call);
        if call.block().is_some_and(|b| b.as_block_node().is_some())
            || !matches!(
                (call.name().as_slice(), args.len()),
                (b"Integer" | b"BigDecimal" | b"Complex" | b"Rational", 1..=2) | (b"Float", 1)
            )
            || call
                .receiver()
                .is_some_and(|r| !ast::is_global_const(&r, &["Kernel"]))
        {
            return;
        }
        if args.iter().any(|arg| exception_pair(arg, file)) {
            return;
        }
        let mut arguments = args
            .iter()
            .map(|n| source(n.location().as_slice(), file))
            .collect::<Vec<_>>();
        arguments.push("exception: false".into());
        let mut prefer = format!(
            "{}({})",
            String::from_utf8_lossy(call.name().as_slice()),
            arguments.join(", ")
        );
        if let Some(receiver) = call.receiver() {
            prefer = format!(
                "{}{}{prefer}",
                source(receiver.location().as_slice(), file),
                source(call.call_operator_loc().unwrap().as_slice(), file)
            );
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("Use `{prefer}` instead."),
            correction: None,
        });
    }
}
// Prism's BeginNode owns the protected statements and clauses; RuboCop has
// a separate rescue expression beneath kwbegin/ensure/def/block. Inspect only
// the actual callback's immediate parent, not source containment or a root walk.
fn rescued_expression<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
) -> Option<(Node<'pr>, usize, usize)> {
    if let Some(r) = node.as_rescue_modifier_node() {
        r.rescue_expression().as_nil_node()?;
        let mut ancestors = cx.ancestors();
        // Modifier rescue can itself be the sole child of an explicit kwbegin.
        // StatementsNode is transparent in the reference AST; parentheses aren't.
        let wrapper = ancestors
            .next()
            .and_then(|n| n.as_statements_node())
            .filter(|s| s.body().len() == 1)
            .and_then(|_| ancestors.next()?.as_begin_node())
            .filter(|b| {
                b.begin_keyword_loc().is_some()
                    && b.rescue_clause().is_none()
                    && b.ensure_clause().is_none()
            });
        let location = wrapper.map_or_else(|| node.location(), |b| b.location());
        return Some((
            r.expression(),
            location.start_offset(),
            location.end_offset(),
        ));
    }
    let rescue = node.as_rescue_node()?;
    let begin = cx.parent()?.as_begin_node()?;
    if rescue.subsequent().is_some()
        || rescue.reference().is_some()
        || begin
            .else_clause()
            .is_some_and(|e| e.statements().is_some())
    {
        return None;
    }
    let statements = begin.statements()?;
    let mut body = statements.body().iter();
    let expression = body.next()?;
    if body.next().is_some() {
        return None;
    }
    let handler = rescue.statements();
    let nil = handler.as_ref().is_some_and(|s| {
        let mut nodes = s.body().iter();
        nodes.next().is_some_and(|n| n.as_nil_node().is_some()) && nodes.next().is_none()
    });
    let kwbegin = begin.begin_keyword_loc().is_some() && begin.ensure_clause().is_none();
    if kwbegin
        && (nil || handler.is_none())
        && rescue.exceptions().iter().all(|e| {
            matches!(
                e.location().as_slice(),
                b"ArgumentError" | b"TypeError" | b"::ArgumentError" | b"::TypeError"
            )
        })
    {
        return Some((
            expression,
            begin.location().start_offset(),
            begin.location().end_offset(),
        ));
    }
    if !nil || !rescue.exceptions().is_empty() {
        return None;
    }
    let start = expression.location().start_offset();
    // An empty else has a nil AST child but its keyword still belongs to the
    // rescue expression's source range (including implicit def/block bodies).
    let end = begin.else_clause().map_or_else(
        || ast::source_end(node),
        |e| e.else_keyword_loc().end_offset(),
    );
    Some((expression, start, end))
}
fn exception_pair(node: &Node<'_>, file: &SourceFile) -> bool {
    let pairs = if let Some(h) = node.as_hash_node() {
        h.elements()
    } else if let Some(h) = node.as_keyword_hash_node() {
        h.elements()
    } else {
        return false;
    };
    pairs.iter().filter_map(|n| n.as_assoc_node()).any(|p| {
        let key = p.key();
        ast::is_reference_symbol(&key, file)
            && key
                .as_symbol_node()
                .is_some_and(|s| s.unescaped() == b"exception")
    })
}
fn source(bytes: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(bytes)
        .utf8_chunks()
        .map(|c| {
            let mut s = c.valid().to_owned();
            for b in c.invalid() {
                s.push_str(&format!("\\x{b:02X}"));
            }
            s
        })
        .collect::<String>()
        .replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's suppressed_exception_in_number_conversion_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/SuppressedExceptionInNumberConversion")
            .collect()
    }
    #[test]
    fn explicit_begin_nil_or_empty_handlers_and_exact_types() {
        for types in [
            "",
            " ArgumentError",
            " ArgumentError, TypeError",
            " ::ArgumentError, ::TypeError",
        ] {
            for body in ["", "  nil\n"] {
                let text = format!("  begin\n    Integer(arg)\n  rescue{types}\n{body}  end\n");
                let found = offenses(&text);
                assert_eq!(found.len(), 1, "{text}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, 3, if body.is_empty() { 4 } else { 5 })
                );
                assert_eq!(
                    found[0].message,
                    "Use `Integer(arg, exception: false)` instead."
                );
            }
        }
    }
    #[test]
    fn rejects_near_miss_rescues_and_existing_exception_pairs() {
        for text in [
            "Integer(42, exception: false)",
            "Integer(arg, exception: false) rescue nil",
            "BigDecimal(arg, exception: false) rescue nil",
            "Integer(arg, exception: true) rescue nil",
            "Integer(arg) rescue 42",
            "Float(arg, unexpected_arg) rescue nil",
            "Integer() rescue nil",
            "Other::Kernel.Integer(arg) rescue nil",
            "self.Integer(arg) rescue nil",
            "begin; Integer(arg); rescue CustomError; nil; end",
            "begin; Integer(arg); rescue; 42; end",
            "begin; Integer(arg); rescue; nil; else; 42; end",
            "begin; Integer(arg); do_something; rescue; nil; end",
            "begin; Integer(arg); rescue => e; nil; end",
            "begin; Integer(arg); rescue; nil; rescue; nil; end",
            "begin; (Integer(arg)); rescue; nil; end",
            "begin; Integer(arg); rescue; (nil); end",
            "Integer(arg) {} rescue nil",
        ] {
            assert!(offenses(text).is_empty(), "{text}");
        }
    }
    #[test]
    fn modifier_constructors_preserve_receiver_and_arguments() {
        for (method, args) in [
            ("Integer", "arg"),
            ("Integer", "arg, base"),
            ("Float", "arg"),
            ("BigDecimal", "s"),
            ("BigDecimal", "s, n"),
            ("Complex", "s"),
            ("Complex", "r, i"),
            ("Rational", "x"),
            ("Rational", "x, y"),
        ] {
            for receiver in ["", "Kernel.", "Kernel::", "::Kernel::", "::Kernel&."] {
                let source = format!("  {receiver}{method}({args}) rescue nil\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
                assert_eq!(
                    found[0].message,
                    format!("Use `{receiver}{method}({args}, exception: false)` instead.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
}
