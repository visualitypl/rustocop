//! `Layout/RescueEnsureAlignment` — align `rescue` and `ensure` with the
//! start of the line that opens their body. `Layout/BeginEndAlignment` is
//! enabled with the fixed `start_of_line` style, so upstream always anchors
//! on that line, from its first non-blank character.

use ruby_prism::{CallNode, Location, Node};

use super::{
    alignment::{end_indentation, start_line_range},
    indentation_width::in_macro_scope,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct RescueEnsureAlignment;

impl Cop for RescueEnsureAlignment {
    fn name(&self) -> &'static str {
        "Layout/RescueEnsureAlignment"
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
        // Prism gives the modifier form its own node kind, so `on_resbody`'s
        // rescue-modifier token exclusion is structural here.
        let keyword = match node {
            Node::RescueNode { .. } => node.as_rescue_node().unwrap().keyword_loc(),
            Node::EnsureNode { .. } => node.as_ensure_node().unwrap().ensure_keyword_loc(),
            _ => return,
        };
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        // Earlier `rescue` clauses chain the later ones.
        let Some(index) = ancestors
            .iter()
            .position(|n| n.as_rescue_node().is_none())
            .filter(|&i| ancestors[i].as_begin_node().is_some())
        else {
            return;
        };
        let Some((anchor, ending)) = alignment(file, &ancestors[index..], &keyword) else {
            return;
        };
        let range = start_line_range(file, anchor);
        let (line, col) = file.line_col(keyword.start_offset());
        let (align_line, align_col) = file.line_col(range.start);
        if col == align_col || line == align_line {
            return;
        }
        let source = &file.bytes[range.start..ending.max(range.start)];
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = ancestors.last().copied().unwrap_or(node);
                let indentation = " ".repeat(align_col as usize - 1);
                end_indentation(file, root, keyword.start_offset(), &indentation)
            })
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`{}` at {line}, {} is not aligned with `{}` at {align_line}, {}.",
                String::from_utf8_lossy(keyword.as_slice()),
                col - 1,
                String::from_utf8_lossy(source),
                align_col - 1,
            ),
            line,
            end_line: line,
            col,
        });
    }
}

/// `alignment_node` and `alignment_source`: the offset whose line anchors the
/// clause and the end of the reported source, or `None` when upstream skips
/// the clause. `chain` starts at the `BeginNode` holding the clause and
/// continues with its ancestors, innermost first.
fn alignment(
    file: &SourceFile,
    chain: &[&Node<'_>],
    keyword: &Location<'_>,
) -> Option<(usize, usize)> {
    let begin = chain[0].as_begin_node()?;
    if let Some(begin_keyword) = begin.begin_keyword_loc() {
        return Some((
            chain[0].location().start_offset(),
            begin_keyword.end_offset(),
        ));
    }
    // The owner in RuboCop's AST: a definition, class-like body, or a block
    // node, which is the call carrying the block (Prism nests the block in
    // its call) or a lambda.
    let (owner, rest, ending) = match chain.get(1)? {
        Node::DefNode { .. } => {
            let def = chain[1].as_def_node().unwrap();
            (chain[1], &chain[2..], def.name_loc().end_offset())
        }
        Node::ClassNode { .. } => {
            let class = chain[1].as_class_node().unwrap();
            (
                chain[1],
                &chain[2..],
                class.constant_path().location().end_offset(),
            )
        }
        Node::ModuleNode { .. } => {
            let module = chain[1].as_module_node().unwrap();
            (
                chain[1],
                &chain[2..],
                module.constant_path().location().end_offset(),
            )
        }
        Node::SingletonClassNode { .. } => {
            let sclass = chain[1].as_singleton_class_node().unwrap();
            (
                chain[1],
                &chain[2..],
                sclass.expression().location().end_offset(),
            )
        }
        Node::LambdaNode { .. } => {
            let lambda = chain[1].as_lambda_node().unwrap();
            if aligned_with_line_break_method(file, chain[1], lambda.opening_loc(), keyword) {
                return None;
            }
            (chain[1], &chain[2..], lambda.opening_loc().end_offset())
        }
        Node::BlockNode { .. } => {
            let block = chain[1].as_block_node().unwrap();
            let call = chain.get(2)?;
            if aligned_with_line_break_method(file, call, block.opening_loc(), keyword) {
                return None;
            }
            (*call, &chain[3..], block.opening_loc().end_offset())
        }
        _ => return None,
    };
    let start = owner.location().start_offset();
    if let Some((wrapper, ending)) = assignment(owner, rest)
        && file.line_of(wrapper) == file.line_of(start)
    {
        return Some((wrapper, ending));
    }
    if owner.as_def_node().is_some()
        && let Some((call, index)) = parent_send(owner, rest)
        && access_modifier(&call, rest[index], &rest[index + 1..])
    {
        let ending = call
            .receiver()
            .map_or(ending, |receiver| receiver.location().end_offset());
        return Some((call.location().start_offset(), ending));
    }
    Some((start, ending))
}

/// `aligned_with_line_break_method?`: a clause in a block whose `do` shares
/// a line with the call's dot or selector may align with either of them.
/// Calls without a selector (`super`, `->`) use the expression start, as the
/// parser gem's keyword map does.
fn aligned_with_line_break_method(
    file: &SourceFile,
    send: &Node<'_>,
    opening: Location<'_>,
    keyword: &Location<'_>,
) -> bool {
    let do_line = file.line_of(opening.start_offset());
    let column = file.line_col(keyword.start_offset()).1;
    let at = |offset: usize| file.line_col(offset) == (do_line, column);
    match send.as_call_node() {
        Some(call) => {
            call.call_operator_loc()
                .is_some_and(|dot| at(dot.start_offset()))
                || call
                    .message_loc()
                    .is_some_and(|selector| at(selector.start_offset()))
        }
        None => at(send.location().start_offset()),
    }
}

/// `assignment_node`: the owner's parent when it is an assignment of the
/// owner, as (start, end of `alignment_source`).
fn assignment(owner: &Node<'_>, rest: &[&Node<'_>]) -> Option<(usize, usize)> {
    let parent = rest.first()?;
    let start = parent.location().start_offset();
    let value = |v: Node<'_>| v.location().start_offset() == owner.location().start_offset();
    // Variable and constant writes, including operator, `||=` and `&&=`
    // forms: the written name (`casgn`'s name ends with its path).
    macro_rules! written {
        ($ending:ident: $($as:ident),*) => {
            $(if let Some(n) = parent.$as() {
                return value(n.value()).then(|| (start, $ending!(n)));
            })*
        };
    }
    macro_rules! name {
        ($n:ident) => {
            $n.name_loc().end_offset()
        };
    }
    macro_rules! target {
        ($n:ident) => {
            $n.target().location().end_offset()
        };
    }
    written!(name:
        as_local_variable_write_node,
        as_local_variable_operator_write_node,
        as_local_variable_or_write_node,
        as_local_variable_and_write_node,
        as_instance_variable_write_node,
        as_instance_variable_operator_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_and_write_node,
        as_class_variable_write_node,
        as_class_variable_operator_write_node,
        as_class_variable_or_write_node,
        as_class_variable_and_write_node,
        as_global_variable_write_node,
        as_global_variable_operator_write_node,
        as_global_variable_or_write_node,
        as_global_variable_and_write_node,
        as_constant_write_node,
        as_constant_operator_write_node,
        as_constant_or_write_node,
        as_constant_and_write_node
    );
    written!(target:
        as_constant_path_write_node,
        as_constant_path_operator_write_node,
        as_constant_path_or_write_node,
        as_constant_path_and_write_node
    );
    // Attribute and index operator writes: upstream raises on
    // `node.child_nodes.first.loc.name`; the receiver is reported instead,
    // as for plain attribute writes.
    macro_rules! receiver {
        ($($as:ident),*) => {
            $(if let Some(n) = parent.$as() {
                let receiver = n.receiver().map(|r| r.location().end_offset());
                return value(n.value()).then_some(receiver).flatten().map(|e| (start, e));
            })*
        };
    }
    receiver!(
        as_call_operator_write_node,
        as_call_or_write_node,
        as_call_and_write_node,
        as_index_operator_write_node,
        as_index_or_write_node,
        as_index_and_write_node
    );
    if let Some(n) = parent.as_multi_write_node() {
        if !value(n.value()) {
            return None;
        }
        // `masgn`: its left-hand side, with parentheses when present.
        let last = match n.rparen_loc() {
            Some(rparen) => rparen.end_offset(),
            None => n
                .lefts()
                .iter()
                .chain(n.rest())
                .chain(n.rights().iter())
                .filter(|t| t.as_implicit_rest_node().is_none())
                .map(|t| t.location().end_offset())
                .max()?,
        };
        return Some((start, last));
    }
    // `obj.attr = value`, `obj[key] = value`: a setter `send`, reported to
    // the end of its receiver.
    let call = parent
        .as_arguments_node()
        .and(rest.get(1))
        .and_then(|n| n.as_call_node())
        .filter(|c| c.is_attribute_write())?;
    Some((
        call.location().start_offset(),
        call.receiver()?.location().end_offset(),
    ))
}

/// The definition's parent in RuboCop's AST when that is a method call, with
/// the call's index in `rest`.
fn parent_send<'pr>(owner: &Node<'pr>, rest: &[&Node<'pr>]) -> Option<(CallNode<'pr>, usize)> {
    match rest.first()? {
        Node::ArgumentsNode { .. } => Some((rest.get(1)?.as_call_node()?, 1)),
        Node::CallNode { .. } => {
            let call = rest[0].as_call_node()?;
            call.receiver()
                .is_some_and(|r| r.location().start_offset() == owner.location().start_offset())
                .then_some((call, 0))
        }
        _ => None,
    }
}

/// `access_modifier?`: a receiverless `public`/`protected`/`private`/
/// `module_function` macro with arguments, or `public_class_method` /
/// `private_class_method` with any receiver. `above` holds the call's
/// ancestors, innermost first.
fn access_modifier<'pr>(call: &CallNode<'pr>, node: &Node<'pr>, above: &[&Node<'pr>]) -> bool {
    match call.name().as_slice() {
        b"public_class_method" | b"private_class_method" => true,
        b"public" | b"protected" | b"private" | b"module_function" => {
            call.receiver().is_none()
                && call.arguments().is_some()
                && in_macro_scope(node, above.iter().copied())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/rescue_ensure_alignment_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The
    //! spec's examples without a `Layout/BeginEndAlignment` configuration, with
    //! it disabled, with its `begin` style or with tab indentation are replayed
    //! at the fixed defaults (BeginEndAlignment enabled, `start_of_line`,
    //! spaces); the findings and single-pass corrections match the fixed
    //! replay recorded in the pinned RuboCop reference run.

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::Severity,
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/RescueEnsureAlignment")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    /// Every pinned example without an offense at the fixed defaults.
    const ACCEPTED: &[&str] = &[
        // 5 accepts the modifier form
        "test rescue nil",
        // 415 accepts end being misaligned
        "def method1\n  'foo'\nend\n\ndef method2\n  'bar'\nrescue\n  'baz' end\n",
        // 428 accepts rescue and ensure on the same line
        "begin; puts 1; rescue; ensure; puts 2; end",
        // 432 accepts correctly aligned rescue
        "begin\n  something\nrescue\n  error\nend\n",
        // 442 accepts correctly aligned ensure
        "begin\n  something\nensure\n  error\nend\n",
        // 452 accepts correctly aligned rescue/ensure with def
        "def foo\n  something\nrescue StandardError\n  handle_error\nensure\n  error\nend\n",
        // 464 accepts correctly aligned rescue/ensure with def with no body
        "def foo\nrescue StandardError\n  handle_error\nensure\n  error\nend\n",
        // 485 accepts aligned rescue in do-end block
        "[1, 2, 3].each do |el|\n  el.to_s\nrescue StandardError => _exception\n  next\nend\n",
        // 495 accepts aligned rescue in do-end block with `.()` call
        "foo.() do |el|\n  el.to_s\nrescue StandardError => _exception\n  next\nend\n",
        // 505 accepts aligned `rescue` with do-end block that line break with leading dot for method cal
        "[1, 2, 3]\n  .each do |el|\n    el.to_s\n  rescue StandardError => _exception\n    next\n  end\n",
        // 537 registers an offense and corrects unindented `rescue` with do-end block that line break wi
        "[1, 2, 3]\n  .each do |el|\n    el.to_s\nrescue StandardError => _exception\n    next\n  end\n",
        // 558 accepts aligned `rescue` with do-end block that line break with trailing dot for method ca
        "[1, 2, 3].\n  each do |el|\n    el.to_s\n  rescue StandardError => _exception\n    next\n  end\n",
        // 590 registers an offense and corrects unindented `rescue` with do-end block that line break wi
        "[1, 2, 3].\n  each do |el|\n    el.to_s\nrescue StandardError => _exception\n    next\n  end\n",
        // 611 accepts aligned `ensure` with do-end block that line break with leading dot for method cal
        "[1, 2, 3]\n  .each do |el|\n    el.to_s\n  ensure\n    next\n  end\n",
        // 643 registers an offense and corrects unindented `ensure` with do-end block that line break wi
        "[1, 2, 3]\n  .each do |el|\n    el.to_s\nensure\n    next\n  end\n",
        // 664 accepts aligned `ensure` with do-end block that line break with trailing dot for method ca
        "[1, 2, 3].\n  each do |el|\n    el.to_s\n  ensure\n    next\n  end\n",
        // 696 registers an offense and corrects unindented `ensure` with do-end block that line break wi
        "[1, 2, 3].\n  each do |el|\n    el.to_s\nensure\n    next\n  end\n",
        // 717 accepts aligned rescue do-end block assigned to local variable
        "result = [1, 2, 3].map do |el|\n  el.to_s\nrescue StandardError => _exception\n  next\nend\n",
        // 727 accepts aligned rescue in do-end block assigned to instance variable
        "@instance = [].map do |_|\nrescue StandardError => _\nend\n",
        // 735 accepts aligned rescue in do-end block assigned to class variable
        "@@class = [].map do |_|\nrescue StandardError => _\nend\n",
        // 743 accepts aligned rescue in do-end block assigned to global variable
        "$global = [].map do |_|\nrescue StandardError => _\nend\n",
        // 751 accepts aligned rescue in do-end block assigned to class
        "CLASS = [].map do |_|\nrescue StandardError => _\nend\n",
        // 759 accepts aligned rescue in do-end block on multi-assignment
        "a, b = [].map do |_|\nrescue StandardError => _\nend\n",
        // 767 accepts aligned rescue in do-end block on operation assignment
        "a += [].map do |_|\nrescue StandardError => _\nend\n",
        // 775 accepts aligned rescue in do-end block on and-assignment
        "a &&= [].map do |_|\nrescue StandardError => _\nend\n",
        // 783 accepts aligned rescue in do-end block on or-assignment
        "a ||= [].map do |_|\nrescue StandardError => _\nend\n",
        // 791 accepts aligned rescue in assigned do-end block starting on newline
        "valid =\n  proc do |bar|\n    baz\n  rescue\n    qux\n  end\n",
        // 802 accepts aligned rescue in do-end block in a method
        "def foo\n  [1, 2, 3].each do |el|\n    el.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        // 52 rescue with begin as RHS of assignment `Layout/BeginEndAlignment` cop is not enabled accep
        "x ||=\n  begin\n    1\n  rescue\n    2\n  end\n",
        // 63 rescue with begin as RHS of assignment `Layout/BeginEndAlignment` cop is not enabled regis
        "x ||= begin\n  1\nrescue\n  2\nend\n",
        // 840 Ruby 2.7 accepts aligned rescue in do-end numbered block in a method
        "def foo\n  [1, 2, 3].each do\n    _1.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        // 879 Ruby 3.4 accepts aligned rescue in do-end `it` block in a method
        "def foo\n  [1, 2, 3].each do\n    it.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        // 1130 when using zsuper with block does not register an offense when correct alignment
        "super do\n  nil\nensure\n  nil\nend\n",
        // 1148 excluded file processes excluded files with issue
        "begin\n  foo\nrescue\n  bar\nend\n",
        // 1188 allows inline access modifier with private modifier behaves like access modifier rescue wi
        "private def test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1219 allows inline access modifier with private modifier behaves like access modifier rescue wi
        "private def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1250 allows inline access modifier with private modifier behaves like access modifier ensure wi
        "private def test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1281 allows inline access modifier with private modifier behaves like access modifier ensure wi
        "private def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1188 allows inline access modifier with private_class_method modifier behaves like access modif
        "private_class_method def test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1219 allows inline access modifier with private_class_method modifier behaves like access modif
        "private_class_method def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1250 allows inline access modifier with private_class_method modifier behaves like access modif
        "private_class_method def test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1281 allows inline access modifier with private_class_method modifier behaves like access modif
        "private_class_method def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1188 allows inline access modifier with public_class_method modifier behaves like access modifi
        "public_class_method def test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1219 allows inline access modifier with public_class_method modifier behaves like access modifi
        "public_class_method def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        // 1250 allows inline access modifier with public_class_method modifier behaves like access modifi
        "public_class_method def test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1281 allows inline access modifier with public_class_method modifier behaves like access modifi
        "public_class_method def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        // 1391 when `Layout/IndentationStyle` enforces tabs does not register an offense when `rescue` is
        "\tbegin\n\t\tfoo\n\trescue\n\t\tbar\n\tend\n",
    ];

    #[test]
    fn pinned_accepted_examples() {
        for source in ACCEPTED {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    /// (source, findings, single-pass correction) for every pinned example
    /// with an offense at the fixed defaults, by spec line.
    /// (line, column, message)
    type Finding = (u32, u32, &'static str);

    const REPORTED: &[(&str, &[Finding], &str)] = &[
        // 475
        (
            "foo = begin\n        bar\n      rescue BazError\n        qux\n      end\n",
            &[(
                3,
                7,
                "`rescue` at 3, 6 is not aligned with `foo = begin` at 1, 0.",
            )],
            "foo = begin\n        bar\nrescue BazError\n        qux\n      end\n",
        ),
        // 516
        (
            "[1, 2, 3]\n  .each do |el|\n    el.to_s\n      rescue StandardError => _exception\n    next\n  end\n",
            &[(
                4,
                7,
                "`rescue` at 4, 6 is not aligned with `[1, 2, 3]\n  .each do` at 1, 0.",
            )],
            "[1, 2, 3]\n  .each do |el|\n    el.to_s\nrescue StandardError => _exception\n    next\n  end\n",
        ),
        // 569
        (
            "[1, 2, 3].\n  each do |el|\n    el.to_s\n      rescue StandardError => _exception\n    next\n  end\n",
            &[(
                4,
                7,
                "`rescue` at 4, 6 is not aligned with `[1, 2, 3].\n  each do` at 1, 0.",
            )],
            "[1, 2, 3].\n  each do |el|\n    el.to_s\nrescue StandardError => _exception\n    next\n  end\n",
        ),
        // 622
        (
            "[1, 2, 3]\n  .each do |el|\n    el.to_s\n      ensure\n    next\n  end\n",
            &[(
                4,
                7,
                "`ensure` at 4, 6 is not aligned with `[1, 2, 3]\n  .each do` at 1, 0.",
            )],
            "[1, 2, 3]\n  .each do |el|\n    el.to_s\nensure\n    next\n  end\n",
        ),
        // 675
        (
            "[1, 2, 3].\n  each do |el|\n    el.to_s\n      ensure\n    next\n  end\n",
            &[(
                4,
                7,
                "`ensure` at 4, 6 is not aligned with `[1, 2, 3].\n  each do` at 1, 0.",
            )],
            "[1, 2, 3].\n  each do |el|\n    el.to_s\nensure\n    next\n  end\n",
        ),
        // 10
        (
            "begin\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `begin` at 1, 0.",
            )],
            "begin\n  something\nrescue\n    error\nend\n",
        ),
        // 42
        (
            "x ||= begin\n        1\n      rescue\n        2\n      end\n",
            &[(
                3,
                7,
                "`rescue` at 3, 6 is not aligned with `x ||= begin` at 1, 0.",
            )],
            "x ||= begin\n        1\nrescue\n        2\n      end\n",
        ),
        // 140
        (
            "x ||= begin\n        1\n      rescue\n        2\n     end\n",
            &[(
                3,
                7,
                "`rescue` at 3, 6 is not aligned with `x ||= begin` at 1, 0.",
            )],
            "x ||= begin\n        1\nrescue\n        2\n     end\n",
        ),
        // 185
        (
            "def test\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `def test` at 1, 0.",
            )],
            "def test\n  something\nrescue\n    error\nend\n",
        ),
        // 206
        (
            "def Test.test\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `def Test.test` at 1, 0.",
            )],
            "def Test.test\n  something\nrescue\n    error\nend\n",
        ),
        // 227
        (
            "class C\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `class C` at 1, 0.",
            )],
            "class C\n  something\nrescue\n    error\nend\n",
        ),
        // 248
        (
            "module M\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `module M` at 1, 0.",
            )],
            "module M\n  something\nrescue\n    error\nend\n",
        ),
        // 269
        (
            "class << self\n  something\n    rescue\n    error\nend\n",
            &[(
                3,
                5,
                "`rescue` at 3, 4 is not aligned with `class << self` at 1, 0.",
            )],
            "class << self\n  something\nrescue\n    error\nend\n",
        ),
        // 290
        (
            "begin\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `begin` at 1, 0.",
            )],
            "begin\n  something\nensure\n    error\nend\n",
        ),
        // 311
        (
            "def test\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `def test` at 1, 0.",
            )],
            "def test\n  something\nensure\n    error\nend\n",
        ),
        // 332
        (
            "def Test.test\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `def Test.test` at 1, 0.",
            )],
            "def Test.test\n  something\nensure\n    error\nend\n",
        ),
        // 353
        (
            "class C\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `class C` at 1, 0.",
            )],
            "class C\n  something\nensure\n    error\nend\n",
        ),
        // 374
        (
            "module M\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `module M` at 1, 0.",
            )],
            "module M\n  something\nensure\n    error\nend\n",
        ),
        // 395
        (
            "class << self\n  something\n    ensure\n    error\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `class << self` at 1, 0.",
            )],
            "class << self\n  something\nensure\n    error\nend\n",
        ),
        // 815
        (
            "def foo\n  [1, 2, 3].each do |el|\n    el.to_s\nrescue StandardError => _exception\n    next\n  end\nend\n",
            &[(
                4,
                1,
                "`rescue` at 4, 0 is not aligned with `[1, 2, 3].each do` at 2, 2.",
            )],
            "def foo\n  [1, 2, 3].each do |el|\n    el.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        ),
        // 853
        (
            "def foo\n  [1, 2, 3].each do\n    _1.to_s\nrescue StandardError => _exception\n    next\n  end\nend\n",
            &[(
                4,
                1,
                "`rescue` at 4, 0 is not aligned with `[1, 2, 3].each do` at 2, 2.",
            )],
            "def foo\n  [1, 2, 3].each do\n    _1.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        ),
        // 892
        (
            "def foo\n  [1, 2, 3].each do\n    it.to_s\nrescue StandardError => _exception\n    next\n  end\nend\n",
            &[(
                4,
                1,
                "`rescue` at 4, 0 is not aligned with `[1, 2, 3].each do` at 2, 2.",
            )],
            "def foo\n  [1, 2, 3].each do\n    it.to_s\n  rescue StandardError => _exception\n    next\n  end\nend\n",
        ),
        // 918
        (
            "result = [1, 2, 3].map do |el|\n  rescue StandardError => _exception\nend\n",
            &[(
                2,
                3,
                "`rescue` at 2, 2 is not aligned with `result` at 1, 0.",
            )],
            "result = [1, 2, 3].map do |el|\nrescue StandardError => _exception\nend\n",
        ),
        // 935
        (
            "@instance = [1, 2, 3].map do |el|\n  rescue StandardError => _exception\nend\n",
            &[(
                2,
                3,
                "`rescue` at 2, 2 is not aligned with `@instance` at 1, 0.",
            )],
            "@instance = [1, 2, 3].map do |el|\nrescue StandardError => _exception\nend\n",
        ),
        // 952
        (
            "@@class = [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(
                2,
                3,
                "`rescue` at 2, 2 is not aligned with `@@class` at 1, 0.",
            )],
            "@@class = [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 969
        (
            "$global = [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(
                2,
                3,
                "`rescue` at 2, 2 is not aligned with `$global` at 1, 0.",
            )],
            "$global = [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 986
        (
            "CLASS = [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(
                2,
                3,
                "`rescue` at 2, 2 is not aligned with `CLASS` at 1, 0.",
            )],
            "CLASS = [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 1003
        (
            "obj.attr = do_something do\n  rescue StandardError\nend\n",
            &[(2, 3, "`rescue` at 2, 2 is not aligned with `obj` at 1, 0.")],
            "obj.attr = do_something do\nrescue StandardError\nend\n",
        ),
        // 1020
        (
            "a, b = [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(2, 3, "`rescue` at 2, 2 is not aligned with `a, b` at 1, 0.")],
            "a, b = [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 1037
        (
            "a += [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(2, 3, "`rescue` at 2, 2 is not aligned with `a` at 1, 0.")],
            "a += [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 1054
        (
            "a &&= [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(2, 3, "`rescue` at 2, 2 is not aligned with `a` at 1, 0.")],
            "a &&= [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 1071
        (
            "a ||= [].map do |_|\n  rescue StandardError => _\nend\n",
            &[(2, 3, "`rescue` at 2, 2 is not aligned with `a` at 1, 0.")],
            "a ||= [].map do |_|\nrescue StandardError => _\nend\n",
        ),
        // 1088
        (
            "valid =\n  proc do |bar|\n    baz\n    rescue\n    qux\n  end\n",
            &[(
                4,
                5,
                "`rescue` at 4, 4 is not aligned with `proc do` at 2, 2.",
            )],
            "valid =\n  proc do |bar|\n    baz\n  rescue\n    qux\n  end\n",
        ),
        // 1111
        (
            "super do\n  nil\n    ensure\n  nil\nend\n",
            &[(
                3,
                5,
                "`ensure` at 3, 4 is not aligned with `super do` at 1, 0.",
            )],
            "super do\n  nil\nensure\n  nil\nend\n",
        ),
        // 1169
        (
            "private def test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `private def test` at 1, 0.",
            )],
            "private def test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1200
        (
            "private def Test.test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `private def Test.test` at 1, 0.",
            )],
            "private def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1231
        (
            "private def test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `private def test` at 1, 0.",
            )],
            "private def test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1262
        (
            "private def Test.test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `private def Test.test` at 1, 0.",
            )],
            "private def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1169
        (
            "private_class_method def test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `private_class_method def test` at 1, 0.",
            )],
            "private_class_method def test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1200
        (
            "private_class_method def Test.test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `private_class_method def Test.test` at 1, 0.",
            )],
            "private_class_method def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1231
        (
            "private_class_method def test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `private_class_method def test` at 1, 0.",
            )],
            "private_class_method def test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1262
        (
            "private_class_method def Test.test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `private_class_method def Test.test` at 1, 0.",
            )],
            "private_class_method def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1169
        (
            "public_class_method def test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `public_class_method def test` at 1, 0.",
            )],
            "public_class_method def test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1200
        (
            "public_class_method def Test.test\n  'foo'\n  rescue\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `public_class_method def Test.test` at 1, 0.",
            )],
            "public_class_method def Test.test\n  'foo'\nrescue\n  'baz'\nend\n",
        ),
        // 1231
        (
            "public_class_method def test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `public_class_method def test` at 1, 0.",
            )],
            "public_class_method def test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1262
        (
            "public_class_method def Test.test\n  'foo'\n  ensure\n  'baz'\nend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `public_class_method def Test.test` at 1, 0.",
            )],
            "public_class_method def Test.test\n  'foo'\nensure\n  'baz'\nend\n",
        ),
        // 1308
        (
            "def test\n  'foo'; rescue; 'baz'\nend\n\ndef test\n  begin\n    'foo'; rescue; 'baz'\n  end\nend\n",
            &[
                (
                    2,
                    10,
                    "`rescue` at 2, 9 is not aligned with `def test` at 1, 0.",
                ),
                (
                    7,
                    12,
                    "`rescue` at 7, 11 is not aligned with `begin` at 6, 2.",
                ),
            ],
            "def test\n  'foo'; rescue; 'baz'\nend\n\ndef test\n  begin\n    'foo'; rescue; 'baz'\n  end\nend\n",
        ),
        // 1328
        (
            "def test\n  'foo'; ensure; 'baz'\nend\n\ndef test\n  begin\n    'foo'; ensure; 'baz'\n  end\nend\n",
            &[
                (
                    2,
                    10,
                    "`ensure` at 2, 9 is not aligned with `def test` at 1, 0.",
                ),
                (
                    7,
                    12,
                    "`ensure` at 7, 11 is not aligned with `begin` at 6, 2.",
                ),
            ],
            "def test\n  'foo'; ensure; 'baz'\nend\n\ndef test\n  begin\n    'foo'; ensure; 'baz'\n  end\nend\n",
        ),
        // 1353
        (
            "\tbegin\n\t\tfoo\n  rescue\n\t\tbar\n\tend\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `begin` at 1, 1.",
            )],
            "\tbegin\n\t\tfoo\n rescue\n\t\tbar\n\tend\n",
        ),
        // 1372
        (
            "\tdef foo\n\t\tbar\n\t\tensure\n\t\tbaz\n\tend\n",
            &[(
                3,
                3,
                "`ensure` at 3, 2 is not aligned with `def foo` at 1, 1.",
            )],
            "\tdef foo\n\t\tbar\n ensure\n\t\tbaz\n\tend\n",
        ),
        // 1401
        (
            "\tx = [begin\n\t\tfoo\n  rescue\n\t\tbar\n\tend]\n",
            &[(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `x = [begin` at 1, 1.",
            )],
            "\tx = [begin\n\t\tfoo\n rescue\n\t\tbar\n\tend]\n",
        ),
    ];

    #[test]
    fn pinned_reported_examples_and_single_pass_corrections() {
        for (before, expected, after) in REPORTED {
            let expected: Vec<_> = expected
                .iter()
                .map(|&(l, c, m)| (l, c, m.to_owned()))
                .collect();
            assert_eq!(positions(before), expected, "{before}");
            if before == after {
                let found = correction_offenses(&RescueEnsureAlignment, before);
                assert!(found.iter().all(|o| o.correction.is_none()), "{before}");
            } else {
                assert_correction(&RescueEnsureAlignment, before, after);
            }
        }
        let found = offenses(REPORTED[0].0);
        assert_eq!(found[0].severity, Severity::Convention);
        assert_eq!(found[0].end_line, found[0].line);
    }

    #[test]
    fn modifier_rescue_and_single_line_bodies_are_not_clauses() {
        for source in [
            "test rescue nil\n",
            "x = foo rescue bar\n",
            "def f = foo rescue bar\n",
            "def f\n  foo rescue bar\nend\n",
            "foo do\n    bar rescue baz\nend\n",
            "begin; a; rescue; b; ensure; c; end\n",
            "x = begin; a\nrescue; b; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn owners_wrappers_and_line_break_methods() {
        for (source, expected) in [
            // Nearest owner wins: explicit begin inside a block.
            (
                "foo do\n  begin\n    a\n  rescue\n  end\nrescue\nend\n",
                vec![],
            ),
            (
                "foo do\n  begin\n    a\n    rescue\n  end\nend\n",
                vec![(
                    4,
                    5,
                    "`rescue` at 4, 4 is not aligned with `begin` at 2, 2.",
                )],
            ),
            // Several clauses, each reported.
            (
                "def f\n  a\n  rescue A\n  rescue B\n  else\n  ensure\nend\n",
                vec![
                    (
                        3,
                        3,
                        "`rescue` at 3, 2 is not aligned with `def f` at 1, 0.",
                    ),
                    (
                        4,
                        3,
                        "`rescue` at 4, 2 is not aligned with `def f` at 1, 0.",
                    ),
                    (
                        6,
                        3,
                        "`ensure` at 6, 2 is not aligned with `def f` at 1, 0.",
                    ),
                ],
            ),
            // Assignment on another line than the owner: the owner's line.
            ("x =\n  def f\n    a\n  rescue\n  end\n", vec![]),
            (
                "x = def f\n  a\n  rescue\nend\n",
                vec![(3, 3, "`rescue` at 3, 2 is not aligned with `x` at 1, 0.")],
            ),
            (
                "A::B = foo do\n  rescue\nend\n",
                vec![(2, 3, "`rescue` at 2, 2 is not aligned with `A::B` at 1, 0.")],
            ),
            (
                "(a, b) = foo do\n  rescue\nend\n",
                vec![(
                    2,
                    3,
                    "`rescue` at 2, 2 is not aligned with `(a, b)` at 1, 0.",
                )],
            ),
            (
                "h[k] = foo do\n  ensure\nend\n",
                vec![(2, 3, "`ensure` at 2, 2 is not aligned with `h` at 1, 0.")],
            ),
            (
                "x = class Foo::Bar\n  rescue\nend\n",
                vec![(2, 3, "`rescue` at 2, 2 is not aligned with `x` at 1, 0.")],
            ),
            (
                "class Foo::Bar < Base\n  a\n  rescue\nend\n",
                vec![(
                    3,
                    3,
                    "`rescue` at 3, 2 is not aligned with `class Foo::Bar` at 1, 0.",
                )],
            ),
            (
                "class << obj\n  a\n  ensure\nend\n",
                vec![(
                    3,
                    3,
                    "`ensure` at 3, 2 is not aligned with `class << obj` at 1, 0.",
                )],
            ),
            // A line-break method aligned with its selector or dot is skipped.
            ("foo.\n  bar do\n  rescue\n  end\n", vec![]),
            ("foo&.bar(1) do\n     rescue\nend\n", vec![]),
            ("  foo.bar(1) do\n      rescue\n  end\n", vec![]),
            (
                "  foo.bar(1) do\n    rescue\n  end\n",
                vec![(
                    2,
                    5,
                    "`rescue` at 2, 4 is not aligned with `foo.bar(1) do` at 1, 2.",
                )],
            ),
            ("foo.() do\n   rescue\nend\n", vec![]),
            (
                "-> do\n  a\n  rescue\nend\n",
                vec![(
                    3,
                    3,
                    "`rescue` at 3, 2 is not aligned with `-> do` at 1, 0.",
                )],
            ),
            (
                "lambda do\n  a\n  ensure\nend\n",
                vec![(
                    3,
                    3,
                    "`ensure` at 3, 2 is not aligned with `lambda do` at 1, 0.",
                )],
            ),
            // Access modifiers need macro scope (or the class-method forms).
            (
                "class C\n  protected def f\n    a\n  rescue\n  end\nend\n",
                vec![],
            ),
            (
                "class C\n  foo def f\n    a\n    rescue\n  end\nend\n",
                vec![(
                    4,
                    5,
                    "`rescue` at 4, 4 is not aligned with `foo def f` at 2, 2.",
                )],
            ),
            (
                "def m\n  private def f\n    a\n    rescue\n  end\nend\n",
                vec![(
                    4,
                    5,
                    "`rescue` at 4, 4 is not aligned with `private def f` at 2, 2.",
                )],
            ),
            (
                "class C\n  module_function def f\n    a\n    rescue\n  end\nend\n",
                vec![(
                    4,
                    5,
                    "`rescue` at 4, 4 is not aligned with `module_function def f` at 2, 2.",
                )],
            ),
            (
                "obj.private_class_method def self.f\n  a\n  rescue\nend\n",
                vec![(3, 3, "`rescue` at 3, 2 is not aligned with `obj` at 1, 0.")],
            ),
        ] {
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(l, c, m)| (l, c, m.to_owned()))
                .collect();
            assert_eq!(positions(source), expected, "{source}");
        }
    }

    #[test]
    fn columns_are_characters_with_bom_crlf_and_tabs() {
        for source in [
            "\u{feff}begin\n  a\nrescue\nend\n",
            "é = begin\n  a\nrescue\nend\n",
            "\tbegin\n\t\ta\n\trescue\n\tend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let before = "\u{feff}begin\r\n  a\r\n  rescue\r\nend\r\n";
        assert_eq!(
            positions(before),
            [(
                3,
                3,
                "`rescue` at 3, 2 is not aligned with `begin` at 1, 0.".into()
            )]
        );
        assert_correction(
            &RescueEnsureAlignment,
            before,
            "\u{feff}begin\r\n  a\r\nrescue\r\nend\r\n",
        );
        let before = "  é = begin\n  a\n\trescue\nend\n";
        assert_eq!(
            positions(before),
            [(
                3,
                2,
                "`rescue` at 3, 1 is not aligned with `é = begin` at 1, 2.".into()
            )]
        );
        assert_correction(
            &RescueEnsureAlignment,
            before,
            "  é = begin\n  a\n  rescue\nend\n",
        );
    }

    #[test]
    fn literal_interiors_keep_their_bytes() {
        let source = "x = <<~H\n  #{begin\n    a\n      rescue\n    end}\nH\n";
        let found = correction_offenses(&RescueEnsureAlignment, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        for source in [
            "x = \"begin\n  rescue\nend\"\n",
            "# begin\n  # rescue\n",
            "x = %w[begin\n  ensure]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn closed_observers_hold_the_edit_but_keep_the_finding() {
        for tail in [
            "p __FILE__",
            "p __dir__",
            "p $0",
            "p $PROGRAM_NAME",
            "p caller",
            "p caller_locations",
            "p method(:f).source_location",
            "SCRIPT_LINES__ = {}",
            "p DATA.read\n__END__\nx",
        ] {
            let source = format!("begin\n  1\n  rescue\nend\n{tail}\n");
            let found = correction_offenses(&RescueEnsureAlignment, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for tail in [
            "p File.read('example.rb')",
            "p STDIN.read",
            "p DATA",
            "system('cat x.rb')",
        ] {
            assert_correction(
                &RescueEnsureAlignment,
                &format!("begin\n  1\n  rescue\nend\n{tail}\n"),
                &format!("begin\n  1\nrescue\nend\n{tail}\n"),
            );
        }
    }

    #[test]
    fn both_directive_spellings_disable_the_finding() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/RescueEnsureAlignment", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\nbegin\n  1\n  ensure\nend\n"),
                    format!("begin\n  1\n  ensure # {prefix}:disable {target}\nend\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&RescueEnsureAlignment, &source).is_empty());
                }
            }
        }
    }
}
