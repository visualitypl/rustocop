//! Operator boundaries from the main Prism AST. Never lex literal/comment text.
use std::ops::Range;

use ruby_prism::{Location, Node};

use crate::{
    cops::walk::{self, NodeVisitor},
    offense::Correction,
    source::SourceFile,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Assignment,
    Other,
    Ignored,
}

pub(super) struct Operator {
    pub range: Range<usize>,
    pub rhs: Range<usize>,
    pub kind: Kind,
    pub no_space: bool,
    pub anchor: bool,
}

pub(super) fn collect(root: &Node<'_>) -> Vec<Operator> {
    let mut collector = Collector {
        operators: Vec::new(),
    };
    walk::walk(&mut collector, root);
    collector
        .operators
        .sort_unstable_by_key(|op| op.range.start);
    collector.operators
}

struct Collector {
    operators: Vec<Operator>,
}
impl Collector {
    fn add(&mut self, loc: Location<'_>, rhs: Node<'_>, kind: Kind, no_space: bool, anchor: bool) {
        self.operators.push(Operator {
            range: loc.start_offset()..loc.end_offset(),
            rhs: rhs.location().start_offset()..rhs.location().end_offset(),
            kind,
            no_space,
            anchor,
        });
    }
}
impl<'pr> NodeVisitor<'pr> for Collector {
    fn enter(&mut self, node: Node<'pr>) {
        macro_rules! writes {
            ($($variant:ident, $as_node:ident, $loc:ident, $kind:ident);* $(;)?) => {
                match &node {
                    $(Node::$variant { .. } => { let n = node.$as_node().unwrap(); self.add(n.$loc(), n.value(), Kind::$kind, false, true); },)*
                    _ => {}
                }
            };
        }
        writes! {
            LocalVariableWriteNode, as_local_variable_write_node, operator_loc, Assignment;
            InstanceVariableWriteNode, as_instance_variable_write_node, operator_loc, Assignment;
            ClassVariableWriteNode, as_class_variable_write_node, operator_loc, Assignment;
            GlobalVariableWriteNode, as_global_variable_write_node, operator_loc, Assignment;
            ConstantWriteNode, as_constant_write_node, operator_loc, Assignment;
            ConstantPathWriteNode, as_constant_path_write_node, operator_loc, Assignment;
            MultiWriteNode, as_multi_write_node, operator_loc, Assignment;
            LocalVariableAndWriteNode, as_local_variable_and_write_node, operator_loc, Assignment;
            LocalVariableOrWriteNode, as_local_variable_or_write_node, operator_loc, Assignment;
            InstanceVariableAndWriteNode, as_instance_variable_and_write_node, operator_loc, Assignment;
            InstanceVariableOrWriteNode, as_instance_variable_or_write_node, operator_loc, Assignment;
            ClassVariableAndWriteNode, as_class_variable_and_write_node, operator_loc, Assignment;
            ClassVariableOrWriteNode, as_class_variable_or_write_node, operator_loc, Assignment;
            GlobalVariableAndWriteNode, as_global_variable_and_write_node, operator_loc, Assignment;
            GlobalVariableOrWriteNode, as_global_variable_or_write_node, operator_loc, Assignment;
            ConstantAndWriteNode, as_constant_and_write_node, operator_loc, Assignment;
            ConstantOrWriteNode, as_constant_or_write_node, operator_loc, Assignment;
            ConstantPathAndWriteNode, as_constant_path_and_write_node, operator_loc, Assignment;
            ConstantPathOrWriteNode, as_constant_path_or_write_node, operator_loc, Assignment;
            CallAndWriteNode, as_call_and_write_node, operator_loc, Assignment;
            CallOrWriteNode, as_call_or_write_node, operator_loc, Assignment;
            IndexAndWriteNode, as_index_and_write_node, operator_loc, Assignment;
            IndexOrWriteNode, as_index_or_write_node, operator_loc, Assignment;
            LocalVariableOperatorWriteNode, as_local_variable_operator_write_node, binary_operator_loc, Other;
            InstanceVariableOperatorWriteNode, as_instance_variable_operator_write_node, binary_operator_loc, Other;
            ClassVariableOperatorWriteNode, as_class_variable_operator_write_node, binary_operator_loc, Other;
            GlobalVariableOperatorWriteNode, as_global_variable_operator_write_node, binary_operator_loc, Other;
            ConstantOperatorWriteNode, as_constant_operator_write_node, binary_operator_loc, Other;
            ConstantPathOperatorWriteNode, as_constant_path_operator_write_node, binary_operator_loc, Other;
            CallOperatorWriteNode, as_call_operator_write_node, binary_operator_loc, Other;
            IndexOperatorWriteNode, as_index_operator_write_node, binary_operator_loc, Other;
        }
        match &node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if let Some(equal) = call.equal_loc() {
                    if let Some(rhs) = call.arguments().and_then(|a| a.arguments().iter().next()) {
                        self.add(equal, rhs, Kind::Other, false, true);
                    }
                    return;
                }
                if call.call_operator_loc().is_some() || call.receiver().is_none() {
                    return;
                }
                let Some(rhs) = call.arguments().and_then(|a| a.arguments().iter().next()) else {
                    return;
                };
                let Some(loc) = call.message_loc() else {
                    return;
                };
                if !matches!(
                    loc.as_slice(),
                    b"+" | b"-"
                        | b"*"
                        | b"/"
                        | b"%"
                        | b"**"
                        | b"=="
                        | b"!="
                        | b"==="
                        | b"<"
                        | b">"
                        | b"<="
                        | b">="
                        | b"<=>"
                        | b"=~"
                        | b"!~"
                        | b"<<"
                        | b">>"
                        | b"&"
                        | b"|"
                        | b"^"
                ) {
                    return;
                }
                let rational = rhs.as_rational_node().is_some();
                if loc.as_slice() == b"/"
                    && rational
                    && call
                        .receiver()
                        .is_some_and(|lhs| lhs.as_integer_node().is_some())
                {
                    return;
                }
                let no_space = loc.as_slice() == b"**" || (loc.as_slice() == b"/" && rational);
                self.add(loc, rhs, Kind::Other, no_space, true);
            }
            Node::AndNode { .. } => {
                let n = node.as_and_node().unwrap();
                self.add(n.operator_loc(), n.right(), Kind::Other, false, true);
            }
            Node::OrNode { .. } => {
                let n = node.as_or_node().unwrap();
                self.add(n.operator_loc(), n.right(), Kind::Other, false, true);
            }
            Node::AssocNode { .. } => {
                let n = node.as_assoc_node().unwrap();
                if let Some(loc) = n.operator_loc().filter(|l| l.as_slice() == b"=>") {
                    self.add(loc, n.value(), Kind::Other, false, true);
                }
            }
            Node::IfNode { .. } => {
                let n = node.as_if_node().unwrap();
                if let Some(loc) = n.then_keyword_loc().filter(|l| l.as_slice() == b"?") {
                    if let Some(rhs) = n.statements() {
                        self.add(loc, rhs.as_node(), Kind::Other, false, true);
                    }
                    if let Some((other, rhs)) = n
                        .subsequent()
                        .and_then(|n| n.as_else_node())
                        .and_then(|other| other.statements().map(|rhs| (other, rhs)))
                    {
                        self.add(
                            other.else_keyword_loc(),
                            rhs.as_node(),
                            Kind::Other,
                            false,
                            true,
                        );
                    }
                }
            }
            Node::ClassNode { .. } => {
                let n = node.as_class_node().unwrap();
                if let (Some(loc), Some(rhs)) = (n.inheritance_operator_loc(), n.superclass()) {
                    self.add(loc, rhs, Kind::Other, false, true);
                }
            }
            Node::SingletonClassNode { .. } => {
                let n = node.as_singleton_class_node().unwrap();
                self.add(n.operator_loc(), node, Kind::Other, false, true);
            }
            Node::DefNode { .. } => {
                let n = node.as_def_node().unwrap();
                if let (Some(loc), Some(rhs)) = (n.equal_loc(), n.body()) {
                    self.add(loc, rhs, Kind::Assignment, false, false);
                }
            }
            Node::OptionalParameterNode { .. } => {
                let n = node.as_optional_parameter_node().unwrap();
                self.add(n.operator_loc(), n.value(), Kind::Ignored, false, false);
            }
            Node::RescueNode { .. } => {
                let n = node.as_rescue_node().unwrap();
                if let (Some(loc), Some(rhs)) = (n.operator_loc(), n.reference()) {
                    self.add(loc, rhs, Kind::Other, false, true);
                }
            }
            Node::AlternationPatternNode { .. } => {
                let n = node.as_alternation_pattern_node().unwrap();
                self.add(n.operator_loc(), node, Kind::Other, false, true);
            }
            Node::CapturePatternNode { .. } => {
                let n = node.as_capture_pattern_node().unwrap();
                self.add(n.operator_loc(), node, Kind::Other, false, true);
            }
            Node::MatchRequiredNode { .. } => {
                let n = node.as_match_required_node().unwrap();
                self.add(n.operator_loc(), node, Kind::Other, false, true);
            }
            _ => {}
        }
    }
}

/// Linear-time line/group indexing: no per-operator scans through a long
/// assignment group or nested block. Predicates mirror RuboCop's
/// PrecedingFollowingAlignment (97f30c4, MIT).
pub(super) struct Alignment<'a> {
    file: &'a SourceFile,
    operators: &'a [Operator],
    first: Vec<Option<usize>>,
    columns: Vec<Vec<usize>>,
    adjacent: Vec<[Option<usize>; 4]>,
    assignments: Vec<[Option<usize>; 2]>,
}
impl<'a> Alignment<'a> {
    pub fn new(file: &'a SourceFile, operators: &'a [Operator]) -> Self {
        let count = file.line_count();
        let mut first = vec![None; count];
        for (index, op) in operators.iter().enumerate() {
            if comparison_token(&file.bytes[op.range.clone()]) {
                first[file.line_of(op.range.start) as usize - 1].get_or_insert(index);
            }
        }
        let columns: Vec<Vec<usize>> = (1..=count)
            .map(|line| {
                let mut start = file.line_start(line as u32);
                let end = start + file.line_text(line as u32).len();
                if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
                    start += 3;
                }
                file.character_offsets(start..end)
                    .chain(std::iter::once(end))
                    .collect()
            })
            .collect();
        let indents: Vec<_> = columns
            .iter()
            .map(|offsets| {
                offsets[..offsets.len() - 1]
                    .iter()
                    .position(|&offset| !file.bytes[offset].is_ascii_whitespace())
            })
            .collect();
        let comments = file.comments();
        let code: Vec<_> = (0..count)
            .map(|i| {
                let Some(indent) = indents[i] else {
                    return false;
                };
                let offset = columns[i][indent];
                let c = comments.partition_point(|c| c.end <= offset);
                !comments.get(c).is_some_and(|c| c.start <= offset)
            })
            .collect();
        let mut adjacent = vec![[None; 4]; count];
        let mut assignments = vec![[None; 2]; count];
        for reverse in [false, true] {
            let mut nearest = None;
            let mut stack: Vec<(usize, usize)> = Vec::new();
            let mut groups: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
            let mut epoch = 0;
            let mut pending_blank = false;
            for n in 0..count {
                let i = if reverse { count - 1 - n } else { n };
                let direction = usize::from(reverse);
                adjacent[i][direction * 2] = nearest;
                if let Some(indent) = indents[i] {
                    if code[i] {
                        while stack.last().is_some_and(|(level, _)| *level > indent) {
                            stack.pop();
                        }
                        adjacent[i][direction * 2 + 1] = stack
                            .last()
                            .filter(|(level, _)| *level == indent)
                            .map(|(_, line)| *line);
                        if stack.last().is_some_and(|(level, _)| *level == indent) {
                            stack.pop();
                        }
                        stack.push((indent, i));
                        nearest = Some(i);
                    }
                    while groups.last().is_some_and(|(level, _)| *level > indent) {
                        groups.pop();
                    }
                    if groups.last().is_none_or(|(level, _)| *level != indent) {
                        groups.push((indent, None));
                    }
                    let anchor = first[i].filter(|&index| {
                        let op = &operators[index];
                        op.anchor && assignment_token(&file.bytes[op.range.clone()])
                    });
                    // Comparisons/defaults/endless separators interrupt groups.
                    if first[i].is_some() && anchor.is_none() {
                        epoch += 1;
                    }
                    let group = groups.last_mut().unwrap();
                    if pending_blank {
                        // In a lookup from this side, the nearest nonblank
                        // line determines whether the blank is at this level.
                        // Defer until here so nested blanks preserve outer
                        // groups in either scan direction.
                        group.1 = None;
                        pending_blank = false;
                    }
                    assignments[i][direction] = group
                        .1
                        .filter(|(_, generation)| *generation == epoch)
                        .map(|(line, _)| line);
                    if anchor.is_some() {
                        group.1 = Some((i, epoch));
                    }
                } else {
                    pending_blank = true;
                }
            }
        }
        Self {
            file,
            operators,
            first,
            columns,
            adjacent,
            assignments,
        }
    }

    pub fn leading(&self, op: &Operator) -> bool {
        let line = self.file.line_of(op.range.start) as usize - 1;
        if op.kind == Kind::Assignment {
            let [previous, next] = self.assignments[line];
            previous.is_some_and(|line| self.equals(&op.range, line))
                || next.is_none_or(|line| self.equals(&op.range, line))
        } else {
            self.adjacent[line]
                .into_iter()
                .flatten()
                .any(|line| self.identical(&op.range, line) || self.equals(&op.range, line))
        }
    }

    pub fn trailing(&self, op: &Operator) -> bool {
        let line = self.file.line_of(op.rhs.start) as usize - 1;
        let column = self.column(op.rhs.start, line);
        self.adjacent[line].into_iter().flatten().any(|other| {
            let offsets = &self.columns[other];
            let word_start = column > 0
                && column + 1 < offsets.len()
                && self.file.bytes[offsets[column - 1]].is_ascii_whitespace()
                && !self.file.bytes[offsets[column]].is_ascii_whitespace();
            word_start || self.identical(&op.rhs, other) || self.equals(&op.rhs, other)
        })
    }

    fn column(&self, offset: usize, line: usize) -> usize {
        self.columns[line].partition_point(|&start| start < offset)
    }

    fn identical(&self, range: &Range<usize>, line: usize) -> bool {
        let column = self.column(range.start, self.file.line_of(range.start) as usize - 1);
        let Some(&start) = self.columns[line].get(column) else {
            return false;
        };
        let end = start + range.len();
        end <= *self.columns[line].last().unwrap()
            && self.file.bytes[start..end] == self.file.bytes[range.clone()]
    }

    fn equals(&self, range: &Range<usize>, line: usize) -> bool {
        let Some(index) = self.first[line] else {
            return false;
        };
        let other = &self.operators[index].range;
        let text = &self.file.bytes[range.clone()];
        let other_text = &self.file.bytes[other.clone()];
        let compatible = (text.ends_with(b"=")
            && (other_text.ends_with(b"=") || other_text == b"<<"))
            || (text == b"<<" && assignment_token(other_text));
        compatible
            && self.column(range.end, self.file.line_of(range.start) as usize - 1)
                == self.column(other.end, line)
    }
}

fn comparison_token(text: &[u8]) -> bool {
    assignment_token(text) || matches!(text, b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<<")
}
fn assignment_token(text: &[u8]) -> bool {
    matches!(
        text,
        b"=" | b"+="
            | b"-="
            | b"*="
            | b"/="
            | b"%="
            | b"**="
            | b"<<="
            | b">>="
            | b"&="
            | b"|="
            | b"^="
            | b"&&="
            | b"||="
    )
}

/// Only horizontal code whitespace is replaceable. Newlines, comments and
/// literal delimiters (including whitespace percent delimiters) stay intact.
pub(super) fn correction(
    file: &SourceFile,
    root: &Node<'_>,
    op: &Operator,
    mut around: Range<usize>,
) -> Option<Correction> {
    let multiline =
        file.bytes[around.clone()].contains(&b'\n') || file.bytes[around.clone()].contains(&b'\r');
    if multiline && op.no_space {
        return None;
    }
    if multiline {
        around.end = op.range.end;
        while file
            .bytes
            .get(around.end)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c))
        {
            around.end += 1;
        }
    }
    let literals = file.literal_token_ranges(root);
    let first = literals.partition_point(|r| r.end <= around.start);
    if literals.get(first).is_some_and(|r| r.start < around.end) {
        return None;
    }
    let text = String::from_utf8_lossy(&file.bytes[op.range.clone()]);
    Some(Correction {
        range: around,
        replacement: if op.no_space {
            text.into_owned()
        } else {
            format!(" {text}{}", if multiline { "" } else { " " })
        },
    })
}

pub(super) fn surrounding(file: &SourceFile, range: &Range<usize>) -> Range<usize> {
    let mut start = range.start;
    while start > 0 && matches!(file.bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    // RangeHelp scans horizontal blanks first, then newlines. It does not
    // cross blanks on a different line. Account for raw CRLF as one newline.
    while start > 0 && file.bytes[start - 1] == b'\n' {
        start -= 1;
        if start > 0 && file.bytes[start - 1] == b'\r' {
            start -= 1;
        }
    }
    let mut end = range.end;
    while file
        .bytes
        .get(end)
        .is_some_and(|b| matches!(b, b' ' | b'\t'))
    {
        end += 1;
    }
    loop {
        if file.bytes.get(end..end + 2) == Some(b"\r\n") {
            end += 2;
        } else if file.bytes.get(end) == Some(&b'\n') {
            end += 1;
        } else {
            break;
        }
    }
    start..end
}
