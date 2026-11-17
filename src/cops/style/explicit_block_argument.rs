//! `Style/ExplicitBlockArgument`: exact wrapper detection with conservative forwarding proofs.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ExplicitBlockArgument;
const MSG: &str =
    "Consider using explicit block argument in the surrounding method's signature over `yield`.";
impl Cop for ExplicitBlockArgument {
    fn name(&self) -> &'static str {
        "Style/ExplicitBlockArgument"
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
        let block = node.as_block_node();
        let (body, parameters, call) = if let Some(block) = &block {
            let Some(call) = cx.parent().filter(|n| {
                matches!(
                    n,
                    Node::CallNode { .. }
                        | Node::SuperNode { .. }
                        | Node::ForwardingSuperNode { .. }
                )
            }) else {
                return;
            };
            (block.body(), block.parameters(), Some(call))
        } else if let Some(lambda) = node.as_lambda_node() {
            // Prism models arrow lambdas separately; upstream's block matcher
            // includes them. Their Proc semantics are not a forwarding proof.
            (lambda.body(), lambda.parameters(), None)
        } else {
            return;
        };
        let Some(body) = body.and_then(|b| b.as_statements_node()) else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let Some(yield_node) = body.body().first().and_then(|n| n.as_yield_node()) else {
            return;
        };
        let mut params = Vec::new();
        if let Some(parameters) = parameters {
            let Some(parameters) = parameters.as_block_parameters_node() else {
                return;
            };
            if let Some(p) = parameters.parameters() {
                params.extend(p.requireds().iter());
                params.extend(p.optionals().iter());
                params.extend(p.rest());
                params.extend(p.posts().iter());
                params.extend(p.keywords().iter());
                params.extend(p.keyword_rest());
                params.extend(p.block().map(|b| b.as_node()));
            }
            params.extend(parameters.locals().iter());
        }
        let args: Vec<_> = yield_node
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if args.len() != params.len()
            || !args.iter().zip(&params).all(|(arg, param)| {
                let name = param
                    .as_required_parameter_node()
                    .map(|p| p.name())
                    .or_else(|| param.as_optional_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_rest_parameter_node().and_then(|p| p.name()))
                    .or_else(|| {
                        param
                            .as_keyword_rest_parameter_node()
                            .and_then(|p| p.name())
                    })
                    .or_else(|| param.as_required_keyword_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_optional_keyword_parameter_node().map(|p| p.name()))
                    .or_else(|| param.as_block_parameter_node().and_then(|p| p.name()))
                    .or_else(|| param.as_block_local_variable_node().map(|p| p.name()));
                name.is_some_and(|name| {
                    arg.as_local_variable_read_node()
                        .is_some_and(|a| a.name().as_slice() == name.as_slice())
                })
            })
        {
            return;
        }
        let Some(def) = cx.ancestors().find_map(|n| n.as_def_node()) else {
            return;
        };
        if def
            .parameters()
            .and_then(|p| p.keyword_rest())
            .is_some_and(|p| p.as_forwarding_parameter_node().is_some())
        {
            return;
        }
        let start = call.unwrap_or(node).location().start_offset();
        let end = node.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: MSG.into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    call.zip(block.as_ref())
                        .and_then(|(call, block)| rewrite(call, block, &def, file))
                })
                .flatten(),
        });
    }
}
/// RuboCop's corrector: the block becomes `&name`, and the method gains a
/// `&name` parameter unless it has one.
fn rewrite(
    call: &Node<'_>,
    block: &ruby_prism::BlockNode<'_>,
    def: &ruby_prism::DefNode<'_>,
    file: &SourceFile,
) -> Option<Correction> {
    let params = def.parameters();
    let existing = params.as_ref().and_then(|p| p.block());
    let name = match &existing {
        Some(b) => std::str::from_utf8(b.name().map_or(&b""[..], |n| n.as_slice())).ok()?,
        None => "block",
    };
    if existing.is_none() && name_taken(def) {
        return None;
    }
    let def_params = params.as_ref().map(parameter_list).unwrap_or_default();
    let zsuper = call.as_forwarding_super_node().is_some();
    if zsuper
        && def_params
            .iter()
            .any(|p| p.as_multi_target_node().is_some())
    {
        return None;
    }
    let send_end = send_end(call)?;
    let mut edits = vec![edit(send_end..block.location().end_offset(), String::new())];
    let insert_after = |edits: &mut Vec<Correction>, last: &Node<'_>| {
        let mut end = last.location().end_offset();
        while file.bytes.get(end) == Some(&b',') {
            end += 1;
        }
        let comma = if file.bytes[..end].ends_with(b",") {
            ""
        } else {
            ","
        };
        edits.push(edit(end..end, format!("{comma} &{name}")));
    };
    // The call.
    if let Some(c) = call.as_call_node() {
        let arguments = crate::cops::ast::call_arguments(&c);
        if let Some(last) = arguments.last() {
            insert_after(&mut edits, last);
        } else if c.name().as_slice() == b"[]" && c.call_operator_loc().is_none() {
            let (open, close) = (c.opening_loc()?, c.closing_loc()?);
            edits.push(edit(
                open.start_offset()..close.end_offset(),
                format!("[&{name}]"),
            ));
        } else {
            call_parentheses(
                &mut edits,
                c.opening_loc(),
                c.closing_loc(),
                send_end,
                &format!("&{name}"),
            );
        }
    } else if let Some(s) = call.as_super_node() {
        match s.arguments().and_then(|a| a.arguments().iter().last()) {
            Some(last) => insert_after(&mut edits, &last),
            None => call_parentheses(
                &mut edits,
                s.lparen_loc(),
                s.rparen_loc(),
                send_end,
                &format!("&{name}"),
            ),
        }
    } else {
        let mut arguments = def_params
            .iter()
            .map(|p| zsuper_argument(p, file))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        arguments.push(format!("&{name}"));
        edits.push(edit(
            send_end..send_end,
            format!("({})", arguments.join(", ")),
        ));
    }
    // The method.
    if existing.is_none() {
        if let Some(last) = def_params.last() {
            insert_after(&mut edits, last);
        } else if let (Some(open), Some(close)) = (def.lparen_loc(), def.rparen_loc()) {
            edits.push(edit(
                open.start_offset()..close.end_offset(),
                format!("(&{name})"),
            ));
        } else {
            let at = def.name_loc().end_offset();
            edits.push(edit(at..at, format!("(&{name})")));
        }
    }
    super::correction::combine(file, edits)
}

fn edit(range: std::ops::Range<usize>, replacement: String) -> Correction {
    Correction { range, replacement }
}

/// RuboCop's `correct_call_node` for a call without arguments.
fn call_parentheses(
    edits: &mut Vec<Correction>,
    open: Option<ruby_prism::Location<'_>>,
    close: Option<ruby_prism::Location<'_>>,
    send_end: usize,
    arguments: &str,
) {
    match (open.filter(|o| o.as_slice() == b"("), close) {
        (Some(open), Some(close)) => {
            edits.push(edit(
                open.start_offset()..close.end_offset(),
                format!("({arguments})"),
            ));
        }
        _ => edits.push(edit(send_end..send_end, format!("({arguments})"))),
    }
}

/// Where parser's send node ends: before the literal block.
fn send_end(call: &Node<'_>) -> Option<usize> {
    if let Some(c) = call.as_call_node() {
        let ends = [
            c.receiver().map(|n| n.location().end_offset()),
            c.message_loc().map(|l| l.end_offset()),
            c.arguments().map(|a| a.location().end_offset()),
            c.closing_loc().map(|l| l.end_offset()),
        ];
        return ends.into_iter().flatten().max();
    }
    if let Some(s) = call.as_super_node() {
        let ends = [
            Some(s.keyword_loc().end_offset()),
            s.arguments().map(|a| a.location().end_offset()),
            s.rparen_loc().map(|l| l.end_offset()),
        ];
        return ends.into_iter().flatten().max();
    }
    Some(call.as_forwarding_super_node()?.location().start_offset() + "super".len())
}

/// The method's parameters in source order.
fn parameter_list<'pr>(p: &ruby_prism::ParametersNode<'pr>) -> Vec<Node<'pr>> {
    let mut list: Vec<Node<'pr>> = Vec::new();
    list.extend(p.requireds().iter());
    list.extend(p.optionals().iter());
    list.extend(p.rest());
    list.extend(p.posts().iter());
    list.extend(p.keywords().iter());
    list.extend(p.keyword_rest());
    list.extend(p.block().map(|b| b.as_node()));
    list.sort_by_key(|n| n.location().start_offset());
    list
}

/// RuboCop's `build_new_arguments_for_zsuper`.
fn zsuper_argument(p: &Node<'_>, file: &SourceFile) -> Option<Option<String>> {
    let name = |n: &[u8]| std::str::from_utf8(n).ok().map(str::to_owned);
    Some(Some(match p {
        Node::RequiredParameterNode { .. } => {
            name(p.as_required_parameter_node()?.name().as_slice())?
        }
        Node::OptionalParameterNode { .. } => {
            name(p.as_optional_parameter_node()?.name().as_slice())?
        }
        Node::RequiredKeywordParameterNode { .. } => {
            let n = name(p.as_required_keyword_parameter_node()?.name().as_slice())?;
            format!("{n}: {n}")
        }
        Node::OptionalKeywordParameterNode { .. } => {
            let n = name(p.as_optional_keyword_parameter_node()?.name().as_slice())?;
            format!("{n}: {n}")
        }
        Node::RestParameterNode { .. } | Node::KeywordRestParameterNode { .. } => {
            let loc = p.location();
            std::str::from_utf8(&file.bytes[loc.start_offset()..loc.end_offset()])
                .ok()?
                .to_owned()
        }
        _ => return Some(None),
    }))
}

/// RuboCop's `block_name_taken?`: a parameter or local assignment named
/// `block` anywhere in the method.
// Pattern captures count as assignments here (parser's match_var
// does not); add the distinction if it ever matters.
fn name_taken(def: &ruby_prism::DefNode<'_>) -> bool {
    struct Taken(bool);
    impl crate::cops::walk::NodeVisitor<'_> for Taken {
        fn enter(&mut self, n: Node<'_>) {
            let name = match &n {
                Node::RequiredParameterNode { .. } => {
                    Some(n.as_required_parameter_node().unwrap().name())
                }
                Node::OptionalParameterNode { .. } => {
                    Some(n.as_optional_parameter_node().unwrap().name())
                }
                Node::RestParameterNode { .. } => n.as_rest_parameter_node().unwrap().name(),
                Node::RequiredKeywordParameterNode { .. } => {
                    Some(n.as_required_keyword_parameter_node().unwrap().name())
                }
                Node::OptionalKeywordParameterNode { .. } => {
                    Some(n.as_optional_keyword_parameter_node().unwrap().name())
                }
                Node::KeywordRestParameterNode { .. } => {
                    n.as_keyword_rest_parameter_node().unwrap().name()
                }
                Node::BlockParameterNode { .. } => n.as_block_parameter_node().unwrap().name(),
                Node::BlockLocalVariableNode { .. } => {
                    Some(n.as_block_local_variable_node().unwrap().name())
                }
                Node::LocalVariableWriteNode { .. } => {
                    Some(n.as_local_variable_write_node().unwrap().name())
                }
                Node::LocalVariableTargetNode { .. } => {
                    Some(n.as_local_variable_target_node().unwrap().name())
                }
                Node::LocalVariableOperatorWriteNode { .. } => {
                    Some(n.as_local_variable_operator_write_node().unwrap().name())
                }
                Node::LocalVariableOrWriteNode { .. } => {
                    Some(n.as_local_variable_or_write_node().unwrap().name())
                }
                Node::LocalVariableAndWriteNode { .. } => {
                    Some(n.as_local_variable_and_write_node().unwrap().name())
                }
                _ => None,
            };
            self.0 |= name.is_some_and(|n| n.as_slice() == b"block");
        }
    }
    let mut taken = Taken(false);
    crate::cops::walk::walk(&mut taken, &def.as_node());
    taken.0
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/explicit_block_argument_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ExplicitBlockArgument")
            .collect()
    }
    #[test]
    fn exact_yield_wrappers_and_locations() {
        for call in [
            "items.something(first_arg) { |i| yield i }",
            "items.something(first_arg) { |i,j| yield i,j }",
            "items.something(a, b,) { |i| yield i }",
            "array&.each do |row|\n    yield row\n  end",
            "callable.() { |row| yield row }",
            "array.each() { |row| yield row }",
            "foo[] { |row| yield row }",
            "foo.[] { |row| yield row }",
            "3.times { yield }",
            "super { yield }",
            "super() { yield }",
            "foo(x) { yield }",
        ] {
            let source = format!("def m\n  {call}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, MSG);
        }
    }
    #[test]
    fn arrow_lambda_identity_wrappers_report_without_corrections() {
        for wrapper in [
            "->(x) { yield x }",
            "->(x, y) do yield x, y end",
            "-> { yield }",
            "->(x = 1) { yield x }",
            "->(*x) { yield x }",
            "->(x:) { yield x }",
        ] {
            let source = format!("def f\n  {wrapper}\nend\n");
            let found = crate::cops::testing::correction_offenses(&ExplicitBlockArgument, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].end_line, 2);
            assert_eq!(found[0].message, MSG);
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "def f; ->(x, y) { yield y, x }; end",
            "def f; ->(x) { yield x, 1 }; end",
            "def f; ->(x) { work; yield x }; end",
            "def f; ->(*x) { yield *x }; end",
            "def f; ->((x, y)) { yield x, y }; end",
            "def f(...); ->(x) { yield x }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn signatures_super_and_nested_defs_still_report() {
        for signature in [
            "m(&blk)",
            "self.m(x, y = 42, *args, **options)",
            "m(x,k:1,j:)",
            "m(x, **nil)",
            "m((a,b))",
            "m(*, **)",
            "m(scope, block = nil)",
            "[]",
        ] {
            let found = offenses(&format!("def {signature}\n  super {{ yield }}\nend"));
            assert_eq!(found.len(), 1, "{signature}");
        }
        let source = "def foo\n  def bar.baz\n    qux { |quux| yield quux }\n  end\nend\n";
        let found = offenses(source);
        assert_eq!((found[0].line, found[0].col), (3, 5));
    }
    #[test]
    fn guarded_core_iterator_canonical_edits_are_atomic() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def f; if block_given?; [1,2].each { |x| yield x }; end; end",
                "def f(&block); if block_given?; [1,2].each(&block); end; end",
            ),
            (
                "def f(); if block_given?; [1].each() { |x| yield x }; end; end",
                "def f(&block); if block_given?; [1].each(&block); end; end",
            ),
            (
                "def f(&blk); if block_given?; [1].each { |x| yield x }; end; end",
                "def f(&blk); if block_given?; [1].each(&blk); end; end",
            ),
        ] {
            assert_correction(&ExplicitBlockArgument, before, after);
        }
    }
    #[test]
    fn no_offense_for_nonidentity_wrappers_or_forwarding() {
        for source in [
            "def m; items.something { |i,j| yield j,i }; end",
            "def m; yield i; end",
            "def m; items.something { |i| work; yield i }; end",
            "def m; items.something { |i,j,k| yield j,k }; end",
            "def m; items.something { |i,j| yield i }; end",
            "render('partial') { yield }",
            "def m(...); bar { |x| yield x }; end",
            "def m(a, ...); super { yield }; end",
            "def m; foo { yield _1 }; end",
            "def m; foo { yield it }; end",
            "def m; foo { |i| yield i, 1 }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
