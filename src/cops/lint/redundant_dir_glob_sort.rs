//! `Lint/RedundantDirGlobSort` — redundant sorting of a single Dir glob.
//!
//! Detection only: upstream's sort removal is explicitly unsafe.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct RedundantDirGlobSort;

impl Cop for RedundantDirGlobSort {
    fn name(&self) -> &'static str {
        "Lint/RedundantDirGlobSort"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism fuses the getter send into compound attribute writes. Its
        // RuboCop parent is the assignment, not the enclosing block body.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.is_safe_navigation() || call.read_name().as_slice() != b"sort" {
                    return;
                }
                (call.receiver(), call.message_loc())
            }};
        }
        let (receiver, selector) = match node {
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            _ => {
                let Some(sort) = node.as_call_node() else {
                    return;
                };
                if sort.name().as_slice() != b"sort"
                    || sort.is_safe_navigation()
                    || sort.block().is_some()
                    || sole_block_body(cx)
                {
                    return;
                }
                (sort.receiver(), sort.message_loc())
            }
        };
        let Some(glob) = receiver.and_then(|receiver| receiver.as_call_node()) else {
            return;
        };
        // The pinned matcher uses the constant's SHORT name, including scoped
        // constants. Literal glob blocks delegate the receiver, but expose their
        // own explicit block parameters rather than the invocation arguments.
        if !matches!(glob.name().as_slice(), b"glob" | b"[]")
            || !glob
                .receiver()
                .is_some_and(|receiver| ast::short_const_name(&receiver).as_deref() == Some("Dir"))
        {
            return;
        }
        if let Some(block) = glob.block().and_then(|b| b.as_block_node()) {
            if explicit_block_parameter_count(block.parameters()) >= 2 {
                return;
            }
        } else {
            let arguments = glob.arguments().map(|a| a.arguments());
            let block_pass = glob
                .block()
                .is_some_and(|b| b.as_block_argument_node().is_some());
            if arguments.as_ref().map_or(0, |a| a.len()) + usize::from(block_pass) >= 2
                || arguments
                    .is_some_and(|a| a.iter().next().is_some_and(|n| n.as_splat_node().is_some()))
            {
                return;
            }
        }
        let Some(selector) = selector else { return };
        let (line, col) = file.reference_line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Remove redundant `sort`.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

// BlockNode#arguments counts top-level explicit parameters and shadow locals.
// Numbered/implicit-it blocks expose no explicit arguments. A rest parameter is
// a restarg, not a splat, and a trailing comma adds no argument in the reference.
fn explicit_block_parameter_count(parameters: Option<Node<'_>>) -> usize {
    let Some(block_parameters) = parameters.and_then(|p| p.as_block_parameters_node()) else {
        return 0;
    };
    let locals = block_parameters.locals().len();
    let Some(parameters) = block_parameters.parameters() else {
        return locals;
    };
    locals
        + parameters.requireds().len()
        + parameters.optionals().len()
        + usize::from(
            parameters
                .rest()
                .is_some_and(|rest| rest.as_implicit_rest_node().is_none()),
        )
        + parameters.posts().len()
        + parameters.keywords().len()
        + usize::from(parameters.keyword_rest().is_some())
        + usize::from(parameters.block().is_some())
}

// RuboCop's any_block_type? parent guard also excludes a single ungrouped
// body expression (ordinary, numbered, implicit-it and lambda blocks). Multiple
// statements have an intervening begin; parentheses/kwbegin stay real parents.
fn sole_block_body(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    let Some(body) = ancestors.next().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    body.body().len() == 1
        && ancestors.next().is_some_and(|parent| {
            parent.as_block_node().is_some() || parent.as_lambda_node().is_some()
        })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_dir_glob_sort_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantDirGlobSort")
            .collect()
    }

    #[test]
    fn pinned_offense_examples_report_only_sort_selector() {
        for (source, line, col) in [
            (
                "Dir.glob(Rails.root.join('test', '*.rb')).sort.each(&method(:require))",
                1,
                43,
            ),
            (
                "::Dir.glob(Rails.root.join('test', '*.rb')).sort.each(&method(:require))",
                1,
                45,
            ),
            ("Dir['./lib/**/*.rb'].sort.each do |file|\nend", 1, 22),
            (
                "Dir\n  .glob(pattern)\n  .sort\n  .each do |file|\n  end",
                3,
                4,
            ),
            (
                "Dir\n  .glob(pattern) # comment\n  .sort\n  .each do |file|\n  end",
                3,
                4,
            ),
            (
                "Dir.glob(pattern). # comment\n  sort.\n  each { |file| file }",
                2,
                3,
            ),
            ("Dir.glob(pattern).\n  sort.\n  each do |file|\n  end", 2, 3),
            ("Dir['./lib/**/*.rb'].sort.each(&method(:require))", 1, 22),
            ("Dir.glob('./lib/**/*.rb').sort", 1, 27),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let o = &found[0];
            assert_eq!((o.line, o.col, o.end_line), (line, col, line), "{source}");
            assert_eq!(o.message, "Remove redundant `sort`.");
            assert!(o.correction.is_none());
        }
    }

    #[test]
    fn short_constant_names_and_shallow_argument_policy() {
        for source in [
            "Other::Dir.glob(pattern).sort",
            "self::Dir[pattern].sort",
            "Dir.glob.sort",
            "Dir.glob(sort: false).sort",
            "Dir.glob(sort: flag).sort",
            "Dir.glob([*paths]).sort",
            "Dir.glob { it }.sort",
            "Dir&.glob(pattern).sort",
            "Dir.glob(pattern).sort(unused)",
            "Dir.glob(pattern).sort += 1",
            "Dir.glob(pattern).sort &&= value",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].message, "Remove redundant `sort`.");
            assert!(found[0].correction.is_none());
        }
        for source in [
            "Dir.glob(pattern, sort: true).sort",
            "Dir.glob(pattern, sort: false).sort",
            "Dir.glob(pattern, sort: flag).sort",
            "Dir.glob(pattern, &block).sort",
            "Dir.glob(*patterns).sort",
            "Dir.glob(pattern)&.sort",
            "Other.glob(pattern).sort",
            "dir = Dir; dir.glob(pattern).sort",
            "(Dir.glob(pattern)).sort",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn literal_glob_blocks_use_explicit_block_parameters_not_call_arguments() {
        for source in [
            "Dir.glob(a, b) {}.sort",
            "Dir.glob(*paths) { |entry| entry }.sort",
            "Dir.glob(a, b) { _2 }.sort",
            "Dir.glob(*paths) { it }.sort",
            "Dir.glob(a, b) { |*entries| entries }.sort",
            "Dir.glob(a, b) { |entry,| entry }.sort",
            "Dir.glob(a, b) { |(a, b)| a }.sort",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.rfind("sort").unwrap() as u32 + 1)
            );
            assert_eq!(found[0].message, "Remove redundant `sort`.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn multiple_explicit_glob_block_parameters_are_exempt() {
        for source in [
            "Dir.glob(pattern) { |a, b| a }.sort",
            "Dir[pattern] { |a, b| a }.sort",
            "Dir.glob(pattern) { |a; b| a }.sort",
            "Dir.glob(pattern) { |; a, b| nil }.sort",
            "Dir.glob(pattern) { |a, *b| a }.sort",
            "Dir.glob(pattern) { |a:, **b| a }.sort",
            "Dir.glob(pattern) { |a = nil, &b| a }.sort",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn literal_block_parent_is_not_just_a_comparator_test() {
        for source in [
            "each { Dir.glob(x).sort }",
            "each { Dir.glob(_1).sort }",
            "each { Dir.glob(it).sort }",
            "-> { Dir.glob(x).sort }",
            "each { (Dir.glob(x).sort) }",
            "each { nil; Dir.glob(x).sort }",
            "each { Dir.glob(x).sort += 1 }",
            "each { consume(Dir.glob(x).sort) }",
        ] {
            let expected = usize::from(
                source.contains("(Dir.glob") || source.contains("nil;") || source.contains("+="),
            );
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "Dir.glob(Rails.root.join('test', '*.rb'), sort: false).each do; end",
            "Dir.glob('./b/*.txt', './a/*.txt').sort.each(&method(:require))",
            "Dir.glob(*path).sort.each(&method(:require))",
            "Dir['./b/*.txt', './a/*.txt'].sort.each(&method(:require))",
            "Dir[*path].sort.each(&method(:require))",
            "sort { |a, b| a <=> b }",
            "Dir.glob('./lib/**/*.rb').sort { |a, b| b <=> a }.each(&method(:require))",
            "Dir.glob('./lib/**/*.rb').sort { _2 <=> _1 }.each(&method(:require))",
            "Dir.glob('./lib/**/*.rb').sort(&comparator).each(&method(:require))",
            "collection.sort",
            "Dir['./lib/**/*.rb'].each do |file|; end",
            "sort.do_something",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
