//! `Lint/OutOfRangeRegexpRef` — flags `$1`..`$9`… references beyond the
//! capture groups of the last regexp matched, which are always `nil`.
//!
//! Matches RuboCop's semantics, which follow the source in order rather than
//! control flow: a file starts with no capture groups; a literal regexp
//! matched with `=~`, `===`, `match`, `grep`, `gsub`, `sub`, `[]`, `slice`,
//! `index`, `scan`, `partition`, `start_with?` and friends (as receiver or
//! first argument, after the call's arguments are evaluated) sets the count;
//! any of those methods called without a literal regexp makes it unknown
//! (nothing is flagged until the next literal match), except calls like
//! `hash[:key]` whose first argument is a non-string basic literal. `when`
//! and `in` clauses set the count from their literal regexp conditions or
//! patterns (the most groups among them), or make it unknown when there are
//! none. Named groups, when present, are the only captures. Interpolated
//! regexps are never counted.
//!
//! One deliberate deviation: RuboCop ignores a bare regexp used as a
//! condition (`if /(a)(b)/`, which matches `$_`) and so reports its valid
//! references; here it sets the count like any other match (an interpolated
//! one makes it unknown).

use ruby_prism::{
    CallAndWriteNode, CallNode, CallOperatorWriteNode, CallOrWriteNode, InNode, IndexAndWriteNode,
    IndexOperatorWriteNode, IndexOrWriteNode, InterpolatedMatchLastLineNode, MatchLastLineNode,
    Node, NumberedReferenceReadNode, Visit, WhenNode,
};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/OutOfRangeRegexpRef` cop.
pub struct OutOfRangeRegexpRef;

/// Methods that match a literal regexp receiver.
const REGEXP_RECEIVER_METHODS: [&[u8]; 3] = [b"=~", b"===", b"match"];

/// Methods that match a literal regexp first argument.
const REGEXP_ARGUMENT_METHODS: [&[u8]; 17] = [
    b"=~",
    b"match",
    b"grep",
    b"gsub",
    b"gsub!",
    b"sub",
    b"sub!",
    b"[]",
    b"slice",
    b"slice!",
    b"index",
    b"rindex",
    b"scan",
    b"partition",
    b"rpartition",
    b"start_with?",
    b"end_with?",
];

impl Cop for OutOfRangeRegexpRef {
    fn name(&self) -> &'static str {
        "Lint/OutOfRangeRegexpRef"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // The count of valid references flows through the file in source
        // order, so one walk from the root tracks it; sources without a
        // `$<digit>` reference are skipped.
        let Some(program) = node.as_program_node() else {
            return;
        };
        let bytes = &file.bytes;
        if !memchr::memchr_iter(b'$', bytes)
            .any(|at| bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            return;
        }
        let mut tracker = Tracker {
            file,
            valid_refs: Some(0),
            offenses: Vec::new(),
        };
        tracker.visit_program_node(&program);
        for (offset, reference, valid) in tracker.offenses {
            let (line, col) = super::array_literal_in_regexp::reference_line_col(offset, file);
            let count = if valid == 0 {
                "no".to_string()
            } else {
                valid.to_string()
            };
            let groups = if valid == 1 { "group" } else { "groups" };
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "${reference} is out of range ({count} regexp capture {groups} detected)."
                ),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Walks the tree in RuboCop's callback order, tracking how many capture
/// groups the last literal match had.
struct Tracker<'a> {
    file: &'a SourceFile,
    /// The number of valid references; `None` when unknown.
    valid_refs: Option<u32>,
    /// (offset, reference number, valid count) of each offense.
    offenses: Vec<(usize, u32, u32)>,
}

impl Tracker<'_> {
    /// RuboCop's `check_regexp`: a literal regexp sets the count; an
    /// interpolated one changes nothing.
    fn check_regexp(&mut self, node: &Node<'_>) {
        if let Some(count) = literal_capture_count(node) {
            self.valid_refs = Some(count);
        }
    }

    /// RuboCop's `after_send`, run once the call's receiver and arguments
    /// have been visited.
    fn after_send(&mut self, name: &[u8], receiver: Option<Node<'_>>, first: Option<Node<'_>>) {
        let argument_method = REGEXP_ARGUMENT_METHODS.contains(&name);
        if !argument_method && !REGEXP_RECEIVER_METHODS.contains(&name) {
            return;
        }
        let receiver = receiver.filter(is_regexp);
        // `hash[:key]`, `array[0]`: a non-string literal cannot be a pattern.
        let unaffected = receiver.is_none()
            && first.as_ref().is_some_and(|first| {
                super::reference_basic_literal::is_basic_literal(first, self.file)
                    && !matches!(
                        first,
                        Node::StringNode { .. }
                            | Node::InterpolatedStringNode { .. }
                            | Node::SourceFileNode { .. }
                    )
            });
        if unaffected {
            return;
        }
        self.valid_refs = None;
        if let Some(first) = first.filter(|first| argument_method && is_regexp(first)) {
            self.check_regexp(&first);
        } else if let Some(receiver) = receiver {
            self.check_regexp(&receiver);
        }
    }

    /// The most capture groups among `nodes` that are literal regexps
    /// (`None` if there are none), as the new count.
    fn set_from_conditions<'pr>(&mut self, nodes: impl Iterator<Item = Node<'pr>>) {
        self.valid_refs = nodes
            .filter(|node| is_regexp(node))
            .filter_map(|node| literal_capture_count(&node))
            .max();
    }
}

impl<'pr> Visit<'pr> for Tracker<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // A literal regexp on the left of `=~` is the parser gem's
        // `match_with_lvasgn`: it counts as soon as it is entered.
        if node.name().as_slice() == b"=~"
            && node.call_operator_loc().is_none()
            && let Some(receiver) = node.receiver()
            && receiver.as_regular_expression_node().is_some()
        {
            self.check_regexp(&receiver);
            ruby_prism::visit_call_node(self, node);
            return;
        }
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
        if let Some(arguments) = node.arguments() {
            self.visit_arguments_node(&arguments);
        }
        // A block argument belongs to the call; a literal block encloses it
        // and runs after it.
        let block = node.block();
        if let Some(block) = block
            .as_ref()
            .filter(|block| block.as_block_argument_node().is_some())
        {
            self.visit(block);
        }
        self.after_send(
            node.name().as_slice(),
            node.receiver(),
            first_argument(node.arguments()),
        );
        if let Some(block) = block
            .as_ref()
            .filter(|block| block.as_block_node().is_some())
        {
            self.visit(block);
        }
    }

    // `a[x] op= v`: the `[]` call runs before the value.
    fn visit_index_operator_write_node(&mut self, node: &IndexOperatorWriteNode<'pr>) {
        self.visit_index_write(
            node.receiver(),
            node.arguments(),
            node.block(),
            node.value(),
        );
    }

    fn visit_index_or_write_node(&mut self, node: &IndexOrWriteNode<'pr>) {
        self.visit_index_write(
            node.receiver(),
            node.arguments(),
            node.block(),
            node.value(),
        );
    }

    fn visit_index_and_write_node(&mut self, node: &IndexAndWriteNode<'pr>) {
        self.visit_index_write(
            node.receiver(),
            node.arguments(),
            node.block(),
            node.value(),
        );
    }

    // `a.b op= v`: the reader call runs before the value.
    fn visit_call_operator_write_node(&mut self, node: &CallOperatorWriteNode<'pr>) {
        self.visit_call_write(node.receiver(), node.read_name().as_slice(), node.value());
    }

    fn visit_call_or_write_node(&mut self, node: &CallOrWriteNode<'pr>) {
        self.visit_call_write(node.receiver(), node.read_name().as_slice(), node.value());
    }

    fn visit_call_and_write_node(&mut self, node: &CallAndWriteNode<'pr>) {
        self.visit_call_write(node.receiver(), node.read_name().as_slice(), node.value());
    }

    fn visit_when_node(&mut self, node: &WhenNode<'pr>) {
        self.set_from_conditions(node.conditions().iter());
        ruby_prism::visit_when_node(self, node);
    }

    fn visit_in_node(&mut self, node: &InNode<'pr>) {
        // Prism wraps a guarded pattern in an `if`/`unless` whose predicate
        // is the guard; the parser gem keeps them apart, pattern first.
        let pattern = node.pattern();
        let (pattern, guard) = match &pattern {
            Node::IfNode { .. } => {
                let guarded = pattern.as_if_node().unwrap();
                (
                    single_statement(guarded.statements()),
                    Some(guarded.predicate()),
                )
            }
            Node::UnlessNode { .. } => {
                let guarded = pattern.as_unless_node().unwrap();
                (
                    single_statement(guarded.statements()),
                    Some(guarded.predicate()),
                )
            }
            _ => (Some(node.pattern()), None),
        };
        if let Some(pattern) = &pattern {
            if is_regexp(pattern) {
                self.valid_refs = literal_capture_count(pattern);
            } else {
                let mut regexps = RegexpCollector { counts: Vec::new() };
                regexps.visit(pattern);
                self.valid_refs = regexps.counts.into_iter().flatten().max();
            }
            self.visit(pattern);
        } else {
            self.valid_refs = None;
        }
        if let Some(guard) = guard {
            self.visit(&guard);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    // A bare regexp condition matches `$_` (see the module docs).
    fn visit_match_last_line_node(&mut self, node: &MatchLastLineNode<'pr>) {
        self.valid_refs = Some(capture_count(
            node.content_loc().as_slice(),
            node.is_extended(),
        ));
    }

    fn visit_interpolated_match_last_line_node(
        &mut self,
        node: &InterpolatedMatchLastLineNode<'pr>,
    ) {
        ruby_prism::visit_interpolated_match_last_line_node(self, node);
        self.valid_refs = None;
    }

    fn visit_numbered_reference_read_node(&mut self, node: &NumberedReferenceReadNode<'pr>) {
        let reference = node.number();
        if let Some(valid) = self.valid_refs
            && reference > valid
        {
            self.offenses
                .push((node.location().start_offset(), reference, valid));
        }
    }
}

impl<'pr> Tracker<'_> {
    fn visit_index_write(
        &mut self,
        receiver: Option<Node<'pr>>,
        arguments: Option<ruby_prism::ArgumentsNode<'pr>>,
        block: Option<ruby_prism::BlockArgumentNode<'pr>>,
        value: Node<'pr>,
    ) {
        if let Some(receiver) = &receiver {
            self.visit(receiver);
        }
        if let Some(arguments) = &arguments {
            self.visit_arguments_node(arguments);
        }
        if let Some(block) = &block {
            self.visit_block_argument_node(block);
        }
        self.after_send(b"[]", receiver, first_argument(arguments));
        self.visit(&value);
    }

    fn visit_call_write(&mut self, receiver: Option<Node<'pr>>, name: &[u8], value: Node<'pr>) {
        if let Some(receiver) = &receiver {
            self.visit(receiver);
        }
        self.after_send(name, receiver, None);
        self.visit(&value);
    }
}

/// Collects the capture counts of every regexp literal in a pattern
/// (`None` for interpolated ones).
struct RegexpCollector {
    counts: Vec<Option<u32>>,
}

impl<'pr> Visit<'pr> for RegexpCollector {
    fn visit_regular_expression_node(&mut self, node: &ruby_prism::RegularExpressionNode<'pr>) {
        self.counts.push(literal_capture_count(&node.as_node()));
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.counts.push(None);
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
    }
}

/// The first argument of a call, if any.
fn first_argument<'pr>(arguments: Option<ruby_prism::ArgumentsNode<'pr>>) -> Option<Node<'pr>> {
    arguments?.arguments().iter().next()
}

/// The only statement of a statements node.
fn single_statement<'pr>(statements: Option<ruby_prism::StatementsNode<'pr>>) -> Option<Node<'pr>> {
    let body = statements?.body();
    let mut iter = body.iter();
    let first = iter.next()?;
    iter.next().is_none().then_some(first)
}

/// RuboCop's `regexp_type?`: a regexp literal, interpolated or not.
fn is_regexp(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
    )
}

/// The capture groups of a non-interpolated regexp literal.
fn literal_capture_count(node: &Node<'_>) -> Option<u32> {
    let regexp = node.as_regular_expression_node()?;
    Some(capture_count(
        regexp.content_loc().as_slice(),
        regexp.is_extended(),
    ))
}

/// The number of capture groups in a regexp source: its named groups if it
/// has any (Ruby then ignores plain groups), else its plain `(...)` groups.
/// Escapes, character classes, comment groups and, in extended mode, `#`
/// comments are skipped; `(?...)` groups other than `(?<name>...)` and
/// `(?'name'...)` do not capture. Extended mode follows inline flags as
/// RuboCop's regexp_parser does: `(?x:...)` / `(?-x:...)` apply within
/// that group, while an isolated `(?x)` / `(?-x)` lasts until an enclosing
/// option group closes (other groups do not restore it, as in Onigmo).
pub fn capture_count(source: &[u8], extended: bool) -> u32 {
    let (named, plain) = capture_counts(source, extended);
    if named > 0 { named } else { plain }
}

/// Named captures for cops which must preserve capture-dependent control flow.
pub(crate) fn has_named_captures(source: &[u8], extended: bool) -> bool {
    capture_counts(source, extended).0 > 0
}

fn capture_counts(source: &[u8], extended: bool) -> (u32, u32) {
    let mut extended = extended;
    // For each open group, the extended flag its `)` restores: only option
    // groups restore one.
    let mut groups: Vec<Option<bool>> = Vec::new();
    let mut named = 0;
    let mut plain = 0;
    let mut i = 0;
    while i < source.len() {
        match source[i] {
            b'\\' => i += 2,
            b'[' => i = skip_class(source, i),
            b'#' if extended => {
                while i < source.len() && source[i] != b'\n' {
                    i += 1;
                }
            }
            b')' => {
                if let Some(Some(outer)) = groups.pop() {
                    extended = outer;
                }
                i += 1;
            }
            b'(' => {
                if source.get(i + 1) != Some(&b'?') {
                    plain += 1;
                    groups.push(None);
                    i += 1;
                    continue;
                }
                match source.get(i + 2) {
                    Some(b'<') if !matches!(source.get(i + 3), Some(b'=' | b'!')) => {
                        named += 1;
                        groups.push(None);
                        i += 3;
                    }
                    Some(b'\'') => {
                        named += 1;
                        groups.push(None);
                        i += 3;
                    }
                    // A comment: skip past its `)`.
                    Some(b'#') => {
                        i += 3;
                        while i < source.len() && source[i] != b')' {
                            i += 1;
                        }
                        i += 1;
                    }
                    // A conditional `(?(1)...)`: skip past the condition.
                    Some(b'(') => {
                        groups.push(None);
                        i += 3;
                        while i < source.len() && source[i] != b')' {
                            i += 1;
                        }
                        i += 1;
                    }
                    _ => match option_group(source, i + 2, extended) {
                        // `(?flags)`: applies until an enclosing option group closes.
                        Some((flag, b')', end)) => {
                            extended = flag;
                            i = end + 1;
                        }
                        // `(?flags:...)`: applies within this group.
                        Some((flag, _, end)) => {
                            groups.push(Some(extended));
                            extended = flag;
                            i = end + 1;
                        }
                        // Lookaround, atomic, absent, ...: a plain group.
                        None => {
                            groups.push(None);
                            i += 2;
                        }
                    },
                }
            }
            _ => i += 1,
        }
    }
    (named, plain)
}

/// Parses inline options (`imx` flags, optionally `-` and more flags)
/// starting at `start`, ended by `)` or `:`. Returns the resulting extended
/// flag, the terminator and its index; `None` if this is not an option group.
fn option_group(source: &[u8], start: usize, extended: bool) -> Option<(bool, u8, usize)> {
    let mut extended = extended;
    let mut on = true;
    let mut i = start;
    loop {
        match *source.get(i)? {
            b'-' if on => on = false,
            b'x' => extended = on,
            b'i' | b'm' => {}
            b'a' | b'd' | b'u' if on => {}
            terminator @ (b')' | b':') if i > start => return Some((extended, terminator, i)),
            _ => return None,
        }
        i += 1;
    }
}

/// The index just past the character class starting at `start` (a `[`),
/// which may nest; a `]` first in a class is literal.
fn skip_class(source: &[u8], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < source.len() {
        match source[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => {
                depth += 1;
                i += 1;
                if source.get(i) == Some(&b'^') {
                    i += 1;
                }
                if source.get(i) == Some(&b']') {
                    i += 1;
                }
                continue;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/out_of_range_regexp_ref_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/OutOfRangeRegexpRef")
            .collect()
    }

    /// (line, col, message) of each offense.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn msg(reference: u32, count: &str, groups: &str) -> String {
        format!("${reference} is out of range ({count} regexp capture {groups} detected).")
    }

    #[test]
    fn dynamic_symbol_arguments_invalidate_capture_state() {
        for literal in [":\"\"", ":''", "%s()", ":\"a\nb\"", ":'a\nb'", ":\"\\\n\""] {
            for call in [
                format!("y[{literal}]"),
                format!("y.slice({literal})"),
                format!("y[{literal}] ||= z"),
            ] {
                let source = format!("/(a)/ =~ x; {call}; $2");
                assert!(found(&source).is_empty(), "{source:?}");
            }
        }
        for literal in [":x", ":\"x\"", ":\"\\u{}\"", ":\"a\\\nb\""] {
            let source = format!("/(a)/ =~ x; y[{literal}];\n$2");
            assert_eq!(
                found(&source),
                [(source.lines().count() as u32, 1, msg(2, "1", "group"))],
                "{source:?}"
            );
        }
    }

    #[test]
    fn flags_references_before_any_regexp() {
        assert_eq!(found("puts $3\n"), vec![(1, 6, msg(3, "no", "groups"))]);
    }

    #[test]
    fn flags_rubocop_spec_offenses_after_matches() {
        for (source, expected) in [
            (
                "/(?<foo>FOO)(?<bar>BAR)/ =~ \"FOOBAR\"\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "/(foo)(bar)/ =~ \"foobar\"\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "/(?<foo>FOO)(BAR)/ =~ \"FOOBAR\"\nputs $2\n",
                (2, 6, msg(2, "1", "group")),
            ),
            ("/bar/ =~ 'foo'\nputs $1\n", (2, 6, msg(1, "no", "groups"))),
            (
                "\"foobar\" =~ /(foo)(bar)/\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "/(foo)(bar)/ === \"foobar\"\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "/(foo)(bar)/.match(\"foobar\")\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "/(foo)(bar)/.match(\"foobar\")\n/(foo)(bar)(baz)/.match?(\"foobarbaz\")\nputs $3\n",
                (3, 6, msg(3, "2", "groups")),
            ),
            (
                "foo[:bar].match?(/(baz)/) && $1\n",
                (1, 30, msg(1, "no", "groups")),
            ),
            (
                "/(foo)/ =~ bar\nbaz[:qux]\nputs $2\n",
                (3, 6, msg(2, "1", "group")),
            ),
            (
                "/(foo)/ =~ bar\nbaz[0]\nputs $2\n",
                (3, 6, msg(2, "1", "group")),
            ),
            (
                "%w[foo foobar].grep(/(foo)/) { $2 }\n",
                (1, 32, msg(2, "1", "group")),
            ),
            (
                "\"foobar\"[/(foo)(bar)/]\nputs $3\n",
                (2, 6, msg(3, "2", "groups")),
            ),
            (
                "if \"foo bar\".gsub(/(\\s+)/, \"\") =~ /foobar/\n  p $1\nend\n",
                (2, 5, msg(1, "no", "groups")),
            ),
            (
                "if \"some : line \" =~ / : (.+)/\n  $2.gsub(/\\s{2}/, \" \")\nend\n",
                (2, 3, msg(2, "1", "group")),
            ),
            (
                "\"some : line \" =~ / : (.+)/\n$1.gsub(/\\s{2}/, \" \")\nputs $1\n",
                (3, 6, msg(1, "no", "groups")),
            ),
        ] {
            assert_eq!(found(source), vec![expected], "for {source:?}");
        }
    }

    #[test]
    fn flags_rubocop_spec_offenses_in_when_and_in_clauses() {
        for (source, expected) in [
            (
                "case \"foobar\"\nwhen /(foo)(bar)/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobarbaz\"\nwhen /(foo)(bar)/, /(bar)baz/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobarbaz\"\nwhen /(foo)(bar)/, /#{var}/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobar\"\nin /(foo)(bar)/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case array\nin [/(foo)(bar)/, /(bar)baz/]\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case hash\nin a: /(foo)(bar)/, b: /(bar)baz/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "a = 1\ncase array\nin [^a, /(foo)(bar)/, /(foo)bar/]\n  $3\nend\n",
                (4, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobar\"\nin /(foo)(bar)/ => x\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case array\nin [/(foo)(bar)/, /(bar)baz/] => x\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobar\"\nin /(foo)(bar)/ | \"foo\"\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case \"foobar\"\nin /(foo)baz/ | /(foo)(bar)/\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
            (
                "case array\nin [/(foo)(bar)/, /#{var}/]\n  $3\nend\n",
                (3, 3, msg(3, "2", "groups")),
            ),
        ] {
            assert_eq!(found(source), vec![expected], "for {source:?}");
        }
    }

    #[test]
    fn flags_rubocop_spec_offenses_for_each_matching_method() {
        for method in ["gsub", "gsub!", "sub", "sub!", "scan"] {
            let source = format!("\"foobar\".{method}(/(foo)(bar)/) {{ $3 }}\n");
            let col = 9 + method.len() as u32 + 18;
            assert_eq!(
                found(&source),
                vec![(1, col, msg(3, "2", "groups"))],
                "for {source:?}"
            );
            for no_offense in [
                format!("\"foobar\".{method}(/(foo)(bar)/) {{ $2 }}\n"),
                format!("some_string.{method}(some_regexp) {{ $3 }}\n"),
            ] {
                assert!(found(&no_offense).is_empty(), "for {no_offense:?}");
            }
        }
        for method in [
            "match",
            "slice",
            "slice!",
            "index",
            "rindex",
            "partition",
            "rpartition",
            "start_with?",
            "end_with?",
        ] {
            for dot in [".", "&."] {
                let source = format!("\"foobar\"{dot}{method}(/(foo)(bar)/)\nputs $3\n");
                assert_eq!(
                    found(&source),
                    vec![(2, 6, msg(3, "2", "groups"))],
                    "for {source:?}"
                );
                for no_offense in [
                    format!("\"foobar\"{dot}{method}(/(foo)(bar)/)\nputs $2\n"),
                    format!("\"foobar\"{dot}{method}(some_regexp)\nputs $3\n"),
                ] {
                    assert!(found(&no_offense).is_empty(), "for {no_offense:?}");
                }
            }
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "/(?<foo>FOO)(?<bar>BAR)/ =~ \"FOOBAR\"\nputs $1\nputs $2\n",
            "/(foo)(bar)/ =~ \"foobar\"\nputs $1\nputs $2\n",
            "/(?<foo>FOO)(BAR)/ =~ \"FOOBAR\"\nputs $1\n",
            "/(foo)(bar)/u =~ \"foobar\"\nputs $1\nputs $2\n",
            "var = '(\\d+)'\n/(?<foo>#{var}*)/ =~ \"12\"\nputs $1\nputs $2\n",
            "foo.match\n",
            "match(bar)\n",
            "foo_bar_regexp = /(foo)(bar)/\nfoo_regexp = /(foo)/\n\nfoo_bar_regexp =~ 'foobar'\nputs $2\n",
            "/(foo)/ =~ bar\nbaz[qux]\nputs $2\n",
            "/(foo)/ =~ bar\nbaz['qux']\nputs $2\n",
            "case \"foobar\"\nwhen /(foo)(bar)/\n  $2\nend\n",
            "case \"foobarbaz\"\nwhen /(foo)(bar)/, /(bar)baz/\n  $2\nend\n",
            "case some_string\nwhen some_regexp\n  $2\nend\n",
            "case \"foobar\"\nin /(foo)(bar)/\n  $2\nend\n",
            "case array\nin [/(foo)(bar)/, /(bar)baz/]\n  $2\nend\n",
            "case hash\nin a: /(foo)(bar)/\n  $2\nend\n",
            "a = 1\ncase array\nin [^a, /(foo)(bar)/]\n  $2\nend\n",
            "case \"foobar\"\nin /(foo)(bar)/ => x\n  $2\nend\n",
            "case array\nin [/(foo)(bar)/, /(bar)baz/] => x\n  $2\nend\n",
            "case \"foobar\"\nin /(foo)(bar)/ | \"foo\"\n  $2\nend\n",
            "case \"foobar\"\nin /(foo)(bar)/ | \"foo\" => x\n  $2\nend\n",
            "case \"foobar\"\nin /(foo)baz/ | /(foo)(bar)/\n  $2\nend\n",
            "case some_string\nin some_regexp\n  $2\nend\n",
            "%w[foo foobar].grep(/(foo)/) { $1 }\n",
            "%w[foo foobar].grep(some_regexp) { $2 }\n",
            "\"foobar\"[/(foo)(bar)/]\nputs $2\n",
            "\"foobar\"[some_regexp]\nputs $3\n",
            "if \"foo bar\".gsub(/\\s+/, \"\") =~ /foo(bar)/\n  p $1\nend\n",
            "if \"some : line \" =~ / : (.+)/\n  $1.gsub(/\\s{2}/, \" \")\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn follows_rubocop_callback_order() {
        // `regexp =~ x` counts before its right-hand side is visited ...
        assert_eq!(found("/(a)/ =~ $2\n"), vec![(1, 10, msg(2, "1", "group"))]);
        // ... a call only once its arguments are, but before its block.
        assert_eq!(
            found("/(a)(b)/ =~ x\ny.gsub(/(c)/, $2) { $2 }\n"),
            vec![(2, 21, msg(2, "1", "group"))]
        );
        // Only the binary operator is the parser gem's `match_with_lvasgn`.
        assert_eq!(
            found("/(a)/.=~($2)\n"),
            vec![(1, 10, msg(2, "no", "groups"))]
        );
        // `a[x] ||= v` calls `[]` before the value.
        assert!(found("a[/(x)(y)/] ||= $2\n").is_empty());
        // A guard runs after its pattern's regexps are counted.
        assert_eq!(
            found("case x\nin /(a)/ if $1\n  $2\nend\n"),
            vec![(3, 3, msg(2, "1", "group"))]
        );
        // Without a regexp condition, a `when` makes the count unknown.
        assert!(found("/(a)/ =~ x\ncase y\nwhen 1\n  $2\nend\n").is_empty());
    }

    #[test]
    fn unknown_matches_suspend_the_check() {
        for source in [
            "(/(a)/) =~ x\n$1\n",
            "/#{x}(a)/ =~ y\n$3\n",
            "x.match(y)\n$5\n",
            "x =~ y\n$5\n",
            "x.index('a')\n$5\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn bare_regexp_condition_sets_the_count() {
        // Deviation from RuboCop, which ignores `match_current_line`.
        assert!(found("if /(a)(b)/\n  $2\nend\n").is_empty());
        assert!(found("if /#{x}(a)/\n  $2\nend\n").is_empty());
        assert_eq!(
            found("if /(a)(b)/\n  $3\nend\n"),
            vec![(2, 3, msg(3, "2", "groups"))]
        );
    }

    #[test]
    fn counts_capture_groups() {
        for (source, extended, count) in [
            ("(a)(b)", false, 2),
            ("(?<a>x)(y)", false, 1),
            ("(?'a'x)(?<b>y)(z)", false, 2),
            ("\\(a\\)", false, 0),
            ("\\\\(a)", false, 1),
            (
                "[(]\\((?:a)(?<=b)(?<!c)(?!d)(?=e)(?>f)(?i:g)(?#(h)",
                false,
                0,
            ),
            ("[a[(]](b)", false, 1),
            ("[](](c)", false, 1),
            ("[^](](c)", false, 1),
            ("(a) # (c)\n(d)", true, 2),
            ("(a) # (c)\n(d)", false, 3),
            ("(?(1)a|b)(c)", false, 1),
            ("(?i)(?x:(a))", false, 1),
            // Inline extended-mode flags (as regexp_parser: only option
            // groups restore the flag; an isolated one leaks out of others).
            ("(?-x)#(a)", true, 1),
            ("(?x) # (a)\n(b)", false, 1),
            ("(?x: # (a)\n)(b)", false, 1),
            ("(?x:a)#(b)", false, 1),
            ("((?x))#(a)", false, 1),
            ("(?-x:(?x))#(a)", false, 1),
            ("(?x:(?-x))#(a)", false, 1),
            ("(?<n>(?x))#(?<m>a)", false, 1),
            ("(?=(?x))#(a)", false, 0),
            ("((?x)#(a)\n)(b)", false, 2),
            ("(?mi-x:#(a))#(b)", true, 1),
            ("(?-mix)#(a)", true, 1),
            ("(?(1)(?x)#(a)\n)(b)", false, 1),
            ("", false, 0),
        ] {
            assert_eq!(
                capture_count(source.as_bytes(), extended),
                count,
                "for {source:?}"
            );
        }
    }

    #[test]
    fn follows_inline_extended_flags() {
        assert!(found("/(?-x)#(a)/x =~ y; puts $1\n").is_empty());
        assert_eq!(
            found("/(?x) # (a)\n(b)/ =~ y; puts $2\n"),
            vec![(2, 17, msg(2, "1", "group"))]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "$0\n",
            "$~\n$&\n",
            "x = $stdout\n",
            "puts '$3'\n",
            "/(a)/ =~ x\nputs $1\n",
            "def m\n  /(a)(b)/ =~ x\n  [$1, $2]\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
