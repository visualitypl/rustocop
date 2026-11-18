//! `Style/SymbolProc` — fixed-default `&:method` for single-call blocks.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SymbolProc;
impl Cop for SymbolProc {
    fn name(&self) -> &'static str {
        "Style/SymbolProc"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    /// Upstream's Style/InverseMethods lists this cop as
    /// autocorrect-incompatible and runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/InverseMethods"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (params, body, start, dispatch, literal) = if let Some(block) = node.as_block_node() {
            let Some(parent) = cx.parent() else { return };
            let name = if let Some(call) = parent.as_call_node() {
                if call.name().as_slice() == b"define_method"
                    || call.receiver().is_some_and(|r| {
                        (r.as_hash_node().is_some()
                            && matches!(call.name().as_slice(), b"select" | b"reject"))
                            || (r.as_array_node().is_some()
                                && matches!(call.name().as_slice(), b"min" | b"max"))
                    })
                {
                    return;
                }
                String::from_utf8_lossy(call.name().as_slice()).into_owned()
            } else if parent.as_super_node().is_some()
                || parent.as_forwarding_super_node().is_some()
            {
                "super".to_owned()
            } else {
                return;
            };
            (
                block.parameters(),
                block.body(),
                block.opening_loc().start_offset(),
                name,
                false,
            )
        } else if let Some(lambda) = node.as_lambda_node() {
            (
                lambda.parameters(),
                lambda.body(),
                node.location().start_offset(),
                "lambda".to_owned(),
                true,
            )
        } else {
            return;
        };
        let Some([param]) = parameter_names::<1>(params, false) else {
            return;
        };
        let Some(call) = single_body(body).and_then(|n| n.as_call_node()) else {
            return;
        };
        if call.is_safe_navigation()
            || call.block().is_some()
            || !ast::call_arguments(&call).is_empty()
            || !call.receiver().is_some_and(|r| local(&r, param))
            || (literal && super::lambda::in_pattern(node, cx))
        {
            return;
        }
        let method = String::from_utf8_lossy(call.name().as_slice());
        let (line, col) = file.line_col(start);
        let end_line = file.last_line_of(node.location());
        let message = if literal {
            let original = if line == end_line {
                format!("`{}`", String::from_utf8_lossy(node.location().as_slice()))
            } else {
                "a lambda literal with a block".to_owned()
            };
            format!("Use `lambda(&:{method})` instead of {original}.")
        } else {
            format!("Pass `&:{method}` as an argument to `{dispatch}` instead of a block.")
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, cx, &method, file))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line,
            correction,
        });
    }
}

/// Upstream's `autocorrect`: `(&:method)` for the block, appended to the
/// arguments when there are any; a lambda literal becomes `lambda(&:method)`.
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    method: &str,
    file: &SourceFile,
) -> Option<Correction> {
    let Some(block) = node.as_block_node() else {
        return Some(Correction {
            range: node.location().start_offset()..node.location().end_offset(),
            replacement: format!("lambda(&:{method})"),
        });
    };
    let parent = cx.parent()?;
    let (arguments, opening) = if let Some(call) = parent.as_call_node() {
        (call.arguments(), call.opening_loc())
    } else if let Some(call) = parent.as_super_node() {
        (call.arguments(), call.lparen_loc())
    } else {
        (None, None)
    };
    let last = arguments.and_then(|a| a.arguments().iter().last());
    // block_range_with_space
    let mut start = match (&last, opening) {
        (None, Some(open)) => open.start_offset(),
        _ => block.opening_loc().start_offset(),
    };
    let bytes = &file.bytes;
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    while start > 0 && bytes[start - 1] == b'\n' {
        start -= 1;
    }
    let block_range = start..block.closing_loc().end_offset();
    let Some(last) = last else {
        return Some(Correction {
            range: block_range,
            replacement: format!("(&:{method})"),
        });
    };
    // autocorrect_with_args: after the last argument and a comma right after it.
    let mut end = last.location().end_offset();
    if bytes.get(end) == Some(&b',') {
        end += 1;
    }
    let comma = if bytes[..end].ends_with(b",") {
        ""
    } else {
        ","
    };
    Correction::from_edits(
        bytes,
        vec![
            (end..end, format!("{comma} &:{method}")),
            (block_range, String::new()),
        ],
    )
}

// Shared with TallyMethod. These match fixed-depth block shapes, not variable
// liveness: Prism has already resolved binding and numbered/implicit parameters.
pub(super) fn single_body(node: Option<Node<'_>>) -> Option<Node<'_>> {
    let statements = node?.as_statements_node()?;
    if statements.body().len() != 1 {
        return None;
    }
    statements.body().iter().next()
}
pub(super) fn local(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|n| n.name().as_slice() == name)
        || (name == b"it" && node.as_it_local_variable_read_node().is_some())
}
pub(super) fn parameter_names<'pr, const N: usize>(
    node: Option<Node<'pr>>,
    trailing_comma: bool,
) -> Option<[&'pr [u8]; N]> {
    let node = node?;
    let mut names = [b"".as_slice(); N];
    if let Some(p) = node.as_numbered_parameters_node() {
        if usize::from(p.maximum()) != N || N > 2 {
            return None;
        }
        names.copy_from_slice(&[b"_1".as_slice(), b"_2".as_slice()][..N]);
        return Some(names);
    }
    if node.as_it_parameters_node().is_some() {
        if N != 1 {
            return None;
        }
        names[0] = b"it";
        return Some(names);
    }
    let block = node.as_block_parameters_node()?;
    if !block.locals().is_empty() {
        return None;
    }
    let p = block.parameters()?;
    if p.requireds().len() != N
        || !p.optionals().is_empty()
        || !p.posts().is_empty()
        || !p.keywords().is_empty()
        || p.keyword_rest().is_some()
        || p.block().is_some()
        || p.rest()
            .is_some_and(|r| !trailing_comma || r.as_implicit_rest_node().is_none())
    {
        return None;
    }
    for (i, parameter) in p.requireds().iter().enumerate() {
        names[i] = parameter.as_required_parameter_node()?.name().as_slice();
    }
    // The pinned SymbolProc also treats a comma in header trivia as
    // destructuring. Only scan accepted required-parameter headers: defaults
    // containing nested blocks must not cause overlapping source scans.
    if !trailing_comma && block.location().as_slice().contains(&b',') {
        return None;
    }
    Some(names)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop symbol_proc_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/SymbolProc")
            .collect()
    }
    #[test]
    fn parameterless_dispatch_reports_block_and_never_corrects() {
        for (s, line, col, end, method, dispatch) in [
            ("coll.map { |e| e.upcase }", 1, 10, 1, "upcase", "map"),
            ("coll&.map { |e| e.upcase }", 1, 11, 1, "upcase", "map"),
            ("method(one, 2) { |x| x.test }", 1, 16, 1, "test", "method"),
            ("super { |x| x.test }", 1, 7, 1, "test", "super"),
            ("super(one, 2) { |x| x.test }", 1, 15, 1, "test", "super"),
            ("something.map { |x| -x }", 1, 15, 1, "-@", "map"),
            (
                "something do |e| # comment\n  e.upcase # comment\nend",
                1,
                11,
                3,
                "upcase",
                "something",
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col, o[0].end_line),
                (line, col, end),
                "{s}"
            );
            assert_eq!(
                o[0].message,
                format!("Pass `&:{method}` as an argument to `{dispatch}` instead of a block.")
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn numbered_it_proc_lambda_and_literal_receivers() {
        for (s, col, method, dispatch) in [
            ("something { _1.foo }", 11, "foo", "something"),
            ("something { it.foo }", 11, "foo", "something"),
            ("proc { |x| x.method }", 6, "method", "proc"),
            ("::Proc.new { _1.method }", 12, "method", "new"),
            ("lambda { it.method }", 8, "method", "lambda"),
            ("[1,2].select { it.foo }", 14, "foo", "select"),
            ("{a: 1}.min { it.foo }", 12, "foo", "min"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, col), "{s}");
            assert_eq!(
                o[0].message,
                format!("Pass `&:{method}` as an argument to `{dispatch}` instead of a block.")
            );
            assert!(o[0].correction.is_none());
        }
        for s in ["->(x) { x.method }", "-> { _1.method }", "-> { it.method }"] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(
                o[0].message,
                format!("Use `lambda(&:method)` instead of `{s}`.")
            );
        }
        let o = offenses("->(arg) do\n  arg.do_something\nend");
        assert_eq!(o.len(), 1);
        assert_eq!(
            o[0].message,
            "Use `lambda(&:do_something)` instead of a lambda literal with a block."
        );
        assert_eq!(o[0].end_line, 3);
    }
    #[test]
    fn patterns_keep_literal_lambdas_except_pinned_expressions_and_guards() {
        for s in [
            "case value; in ->(x) { x.positive? } then 1; end",
            "case value; in [Integer, -> { _1.positive? }] then 1; end",
            "value in ->(x) { x.positive? }",
            "value => -> { it.positive? }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for (s, col) in [
            ("case value\nin ^(->(x) { x.positive? }) then 1\nend", 6),
            (
                "case value\nin Integer if -> { it.positive? } then 1\nend",
                15,
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, col));
        }
    }
    #[test]
    fn arity_binding_and_control_flow_lookalikes() {
        for s in [
            "something { |x, y| x.method }",
            "something { x.method }",
            "something { |x| }",
            "something { |x| x.method; something_else }",
            "something { |x| y.method }",
            "something { |&x| x.call }",
            "something { |*x| x.first }",
            "something { |x,| x.first }",
            "xs.map do |x # ,\n|\n x.foo\nend",
            "something { |(x)| x.first }",
            "something { |x; y| x.first }",
            "something { |x=1| x.first }",
            "something { |x:| x.first }",
            "something { |x| x.method(1) }",
            "something { |x| x&.method }",
            "something { |x| next x.method }",
            "something { |x| break x.method }",
            "something { |x| return x.method }",
            "something { |x| x.method { effect } }",
            "something { _2.first }",
            "something { _1 + _2 }",
            "something { |x| (x.method) }",
            "define_method(:foo) { |foo| foo.bar }",
            "{foo: 42}.select { |item| item.foo }",
            "{foo: 42}.reject { _1.foo }",
            "[1, 2].min { it.foo }",
            "[1, 2].max { |x| x.foo }",
            "y = Y.new; something { |_x| y.call }",
            "something { |x| -> { x.foo } }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
