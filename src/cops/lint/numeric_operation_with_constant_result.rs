//! `Lint/NumericOperationWithConstantResult` — exact constant-result syntax.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumericOperationWithConstantResult;
#[cfg(test)]
thread_local! {
    static DECLARATION_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Cop for NumericOperationWithConstantResult {
    fn name(&self) -> &'static str {
        "Lint/NumericOperationWithConstantResult"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let matches = if let Some(call) = node.as_call_node() {
            let operator = call.name().as_slice();
            if call.is_safe_navigation() || !matches!(operator, b"*" | b"**" | b"/") {
                return;
            }
            let args = ast::call_arguments(&call);
            let [rhs] = args.as_slice() else { return };
            let zero = integer_zero(rhs);
            if operator != b"/" && !zero {
                return;
            }
            let Some(lhs) = call.receiver() else { return };
            let Some(lhs) = bare_call(&lhs, node, cx, file) else {
                return;
            };
            let rhs = (operator == b"/")
                .then(|| bare_call(rhs, node, cx, file))
                .flatten();
            constant_result(&lhs, operator, zero, rhs.as_deref())
        } else if let Some(write) = node.as_local_variable_operator_write_node() {
            let operator = write.binary_operator().as_slice();
            if !matches!(operator, b"*" | b"**" | b"/") {
                return;
            }
            let rhs = write.value();
            let zero = integer_zero(&rhs);
            if operator != b"/" && !zero {
                return;
            }
            let name = if operator != b"/" {
                None
            } else if let Some(read) = rhs.as_local_variable_read_node() {
                Some(file.match_pattern_source(read.name().as_slice()))
            } else if projected_local(&rhs, node, cx, file) {
                Some(file.match_pattern_source(rhs.as_call_node().unwrap().name().as_slice()))
            } else {
                None
            };
            constant_result(
                &file.match_pattern_source(write.name().as_slice()),
                operator,
                zero,
                name.as_deref(),
            )
        } else {
            false
        };
        if !matches {
            return;
        }
        // The operation becomes its result; a compound assignment assigns it.
        let correction = cx.corrections_requested().then(|| {
            let (operator, name) = match node.as_call_node() {
                Some(call) => (call.name().as_slice().to_vec(), None),
                None => {
                    let write = node.as_local_variable_operator_write_node().unwrap();
                    (
                        write.binary_operator().as_slice().to_vec(),
                        Some(String::from_utf8_lossy(write.name().as_slice()).into_owned()),
                    )
                }
            };
            let result = if operator == b"*" { "0" } else { "1" };
            crate::offense::Correction {
                range: node.location().start_offset()..node.location().end_offset(),
                replacement: match name {
                    Some(name) => format!("{name} = {result}"),
                    None => result.to_owned(),
                },
            }
        });
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Numeric operation with a constant result detected.".into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            correction,
        });
    }
}
// Exact node-pattern arity: an argumentless receiverless send, not a local
// read, a call with a literal block, or a value inferred to be numeric.
fn bare_call<'pr>(
    node: &Node<'pr>,
    operation: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
) -> Option<Vec<u8>> {
    let call = node.as_call_node()?;
    (call.receiver().is_none()
        && call.arguments().is_none()
        && call.block().is_none()
        && !projected_local(node, operation, cx, file))
    .then(|| {
        file.match_pattern_source(call.name().as_slice())
            .into_owned()
    })
}
// Only UTF8-MAC's source-buffer composition can change a raw vcall into a
// local. Both operands are shallow matcher leaves: their lexical scope edges
// are exactly the owning operation's NodeContext. VariableForce supplies real
// declarations, not inferred values or a second parse. Explicit calls stay calls.
fn projected_local<'pr>(
    node: &Node<'pr>,
    operation: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let name = call.name().as_slice();
    if !file.match_pattern_composes_identifiers()
        || !call.is_variable_call()
        || name.is_ascii()
        || call.receiver().is_some()
        || call.arguments().is_some()
        || call.block().is_some()
    {
        return false;
    }
    let projected = file.match_pattern_source(name);
    let Some(root) = cx.ancestors().last() else {
        return false;
    };
    let locals = file.visible_locals(root);
    // Index once, retaining raw names for VariableForce's scope/order queries.
    // Deduplicate spellings shared by multiple scopes; unrelated declarations
    // never enter a receiver's lookup bucket.
    let aliases = file.numeric_operation_local_aliases.get_or_init(|| {
        let mut seen = std::collections::HashSet::new();
        let mut aliases = std::collections::HashMap::<Vec<u8>, Vec<Vec<u8>>>::new();
        for alias in locals.declaration_names() {
            #[cfg(test)]
            DECLARATION_VISITS.set(DECLARATION_VISITS.get() + 1);
            if !alias.is_ascii() && seen.insert(alias) {
                aliases
                    .entry(file.match_pattern_source(alias).into_owned())
                    .or_default()
                    .push(alias.to_vec());
            }
        }
        aliases
    });
    aliases.get(projected.as_ref()).is_some_and(|aliases| {
        aliases.iter().any(|alias| {
            #[cfg(test)]
            DECLARATION_VISITS.set(DECLARATION_VISITS.get() + 1);
            alias != name
                && locals.lexically_declared_before_with(alias, operation, cx, |start| {
                    super::non_atomic_file_operation::heredoc_order(file.heredocs(root), start)
                        <= super::non_atomic_file_operation::heredoc_order(
                            file.heredocs(root),
                            node.location().start_offset(),
                        )
                })
        })
    })
}
fn integer_zero(node: &Node<'_>) -> bool {
    let negative = ast::negated_numeric_literal(node);
    negative
        .as_ref()
        .unwrap_or(node)
        .as_integer_node()
        .is_some_and(|n| n.value().to_u32_digits().1.iter().all(|d| *d == 0))
}
fn constant_result(lhs: &[u8], operation: &[u8], zero: bool, rhs: Option<&[u8]>) -> bool {
    (zero && matches!(operation, b"*" | b"**")) || (operation == b"/" && rhs == Some(lhs))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/numeric_operation_with_constant_result_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NumericOperationWithConstantResult")
            .collect()
    }
    #[test]
    fn pinned_operations_and_abbreviated_local_assignments() {
        for expression in [
            "x * 0",
            "x / x",
            "x ** 0",
            "x *= 0",
            "x /= x",
            "x **= 0",
            "x.*(0)",
            "x./(x)",
            "x.**(0)",
            "x() / x()",
            "x * -0",
            "x ** 0x0",
        ] {
            let source = format!("# numeric\n  {expression}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Numeric operation with a constant result detected."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn exact_call_and_local_shapes_not_inferred_numeric_types() {
        for source in [
            "x - x",
            "x -= x",
            "x % x",
            "x %= x",
            "x % 1",
            "x %= 1",
            "x&.*(0)",
            "x&.**(0)",
            "x&./(x)",
            "0 * x",
            "x * 0.0",
            "x * 0r",
            "x * 0i",
            "x * (0)",
            "x * -(0)",
            "x / y",
            "self.x * 0",
            "x(1) * 0",
            "x {} * 0",
            "(x) * 0",
            "@x * 0",
            "x = 1; x * 0",
            "x = 1; x / x",
            "x / x(1)",
            "x / (x)",
            "x / x {}",
            "@x *= 0",
            "self.x *= 0",
            "A **= 0",
            "x[0] *= 0",
            "x *= 0.0",
            "x **= (0)",
            "x /= y",
            "x /= x()",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn reference_buffer_names_use_real_lexical_declarations_not_value_flow() {
        for (a, b) in [("e\u{301}", "é"), ("é", "e\u{301}")] {
            for body in [
                format!("{a} / {b}"),
                format!("{a} /= {b}"),
                format!("{a} = 1; {b} /= {a}"),
                format!("{b} * 0; {a} = 1"),
            ] {
                let found = offenses(&format!("# coding: UTF8-MAC\n{body}\n"));
                assert_eq!(found.len(), 1, "{body}");
                assert_eq!(found[0].line, 2);
                assert_eq!(
                    found[0].message,
                    "Numeric operation with a constant result detected."
                );
                assert!(found[0].correction.is_none());
            }
            for body in [
                format!("{a} = 1; {b} * 0"),
                format!("proc {{ |{a}| {b} * 0 }}"),
                format!("def f({a}); {b} * 0; end"),
                format!("{a} = 1; proc {{ {b} * 0 }}"),
            ] {
                assert!(
                    offenses(&format!("# coding: UTF8-MAC\n{body}\n")).is_empty(),
                    "{body}"
                );
            }
            // No global normalization: UTF-8 names differ, and explicit method
            // calls stay sends even when a matching local exists.
            assert!(offenses(&format!("# coding: UTF-8\n{a} / {b}\n")).is_empty());
            assert_eq!(
                offenses(&format!("# coding: UTF8-MAC\n{a} = 1; {b}() * 0\n")).len(),
                1
            );
        }
    }
    #[test]
    fn unicode_operands_have_bounded_declaration_work() {
        use crate::{cops::run_cops, source::SourceFile};
        for n in [128, 512] {
            let declarations: String = (0..n)
                .map(|i| format!("local_{i} = 1\nlocal_é_{i} = 1\n"))
                .collect();
            for encoding in ["UTF-8", "UTF8-MAC"] {
                for expression in ["é.foo", "é * 1", "é * 0", "é / é"] {
                    let source = format!(
                        "# coding: {encoding}\n{declarations}{}",
                        format!("{expression}\n").repeat(n)
                    );
                    let file = SourceFile::new("bounded.rb".into(), source.into_bytes());
                    let parsed = crate::parser::parse(&file.bytes);
                    assert_eq!(parsed.errors().count(), 0);
                    super::DECLARATION_VISITS.set(0);
                    // A second dispatch must reuse the same source-local index.
                    for _ in 0..2 {
                        let mut out = vec![];
                        assert!(
                            run_cops(
                                &[&super::NumericOperationWithConstantResult],
                                &parsed,
                                &file,
                                &mut out,
                            )
                            .is_empty()
                        );
                        assert_eq!(
                            out.len(),
                            if matches!(expression, "é * 0" | "é / é") {
                                n
                            } else {
                                0
                            }
                        );
                    }
                    let work = super::DECLARATION_VISITS.get();
                    let bound = if encoding == "UTF8-MAC" && matches!(expression, "é * 0" | "é / é")
                    {
                        2 * n
                    } else {
                        0
                    };
                    assert!(work <= bound, "{encoding} {expression}: {work} > {bound}");
                    if bound == 0 {
                        assert!(file.numeric_operation_local_aliases.get().is_none());
                    } else {
                        assert_eq!(file.numeric_operation_local_aliases.get().unwrap().len(), n);
                    }
                }
            }
        }
    }
    #[test]
    fn literal_blocks_belong_to_parent_not_underlying_send() {
        for source in [
            "x.*(0) {}",
            "x./(x) do; end",
            "consume { x * 0 }",
            "-> { x ** 0 }",
            "def f; x /= x; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
