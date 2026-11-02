//! `Lint/DuplicateRegexpCharacterClassElement` — repeated raw regexp set elements.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

pub struct DuplicateRegexpCharacterClassElement;
impl Cop for DuplicateRegexpCharacterClassElement {
    fn name(&self) -> &'static str {
        "Lint/DuplicateRegexpCharacterClassElement"
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
        let Some(regexp) = crate::cops::style::regexp_literal::Regexp::new(node) else {
            return;
        };
        // Pinned RegexpNode#options has no e/s entry: assign_properties rescues
        // that lookup error, leaving parsed_tree nil even for ASCII content.
        // This is a source-parser fact, not a public encoding/version setting.
        if regexp.closing.as_slice()[1..]
            .iter()
            .any(|b| matches!(b, b'e' | b's'))
        {
            return;
        }
        let view = View::new(&regexp, file);
        let ranges = Scanner::new(&view, regexp.closing.as_slice().contains(&b'x')).scan();
        let correctable = cx.corrections_requested() && closed_defined_literal(node, cx, file);
        for range in ranges {
            let raw = view.starts[range.start]..view.ends[range.end - 1];
            let (line, col) = view.positions[range.start];
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Duplicate element inside regexp character class".into(),
                line,
                col,
                end_line: file.line_of(raw.end - 1),
                correction: correctable.then_some(Correction {
                    range: raw,
                    replacement: String::new(),
                }),
            });
        }
    }
}

// A real, consumed expression, NOT an unreachable branch: Ruby's defined?
// recognizes a direct regexp literal without constructing/evaluating it. Ordinary
// regexps change #source and native -w duplicate-range warnings after deletion.
// Restrict corrections to one closed top-level [defined?(STATIC_REGEXP)] with
// plain ASCII set text. No calls, binding mutations, delayed execution, other
// statements, interpolation, comments inside the regexp or source/DATA observers.
// Main-parse source/position/code-object/heap/heredoc guards still apply centrally.
fn closed_defined_literal(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let Some(regexp) = node.as_regular_expression_node() else {
        return false;
    };
    let content = regexp.content_loc();
    if file.data_start().is_some()
        || content.as_slice().iter().filter(|&&b| b == b'[').count() != 1
        || content.as_slice().iter().filter(|&&b| b == b']').count() != 1
        || !content
            .as_slice()
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"[]^-+*?".contains(b))
    {
        return false;
    }
    let mut ancestors = cx.ancestors();
    if ancestors
        .next()
        .is_none_or(|n| n.as_defined_node().is_none())
    {
        return false;
    }
    let Some(array) = ancestors.next().and_then(|n| n.as_array_node()) else {
        return false;
    };
    if array.elements().len() != 1 {
        return false;
    }
    let Some(statements) = ancestors.next().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    ancestors
        .next()
        .is_some_and(|n| n.as_program_node().is_some())
        && ancestors.next().is_none()
        && statements.body().len() == 1
}

// RuboCop parses source text (not decoded regexp values), blanking each
// interpolation character. Project with the existing pinned buffer facts and
// retain original byte boundaries: neither legacy multibyte trails nor Ruby
// expressions inside interpolation can become regexp syntax. No candidate parse.
pub(super) struct View {
    pub(super) bytes: Vec<u8>,
    pub(super) starts: Vec<usize>,
    pub(super) ends: Vec<usize>,
    holes: Vec<usize>,
    pub(super) positions: Vec<(u32, u32)>,
    position: (u32, u32),
    fingerprints: Vec<u64>,
    powers: Vec<u64>,
}
impl View {
    fn empty(position: (u32, u32)) -> Self {
        Self {
            bytes: vec![],
            starts: vec![],
            ends: vec![],
            holes: vec![0],
            positions: vec![],
            position,
            fingerprints: vec![0],
            powers: vec![1],
        }
    }
    fn new(regexp: &crate::cops::style::regexp_literal::Regexp<'_>, file: &SourceFile) -> Self {
        let position =
            super::duplicate_match_pattern::buffer_line_col(regexp.opening.end_offset(), file);
        let mut view = Self::empty(position);
        let mut previous = regexp.opening.end_offset();
        let mut shifted = false;
        for fragment in &regexp.fragments {
            let shorthand = file.bytes.get(previous) == Some(&b'#')
                && matches!(file.bytes.get(previous + 1), Some(b'@' | b'$'));
            // Upstream only blanks begin nodes, not #@ivar/#$gvar/#@@cvar.
            // Those children expose source WITHOUT '#'; retain that exact
            // raw-buffer quirk, including the consequent projected locations.
            shifted |= shorthand;
            view.position.1 += u32::from(shorthand);
            view.append(
                file,
                previous + usize::from(shorthand)..fragment.start_offset(),
                !shorthand,
            );
            view.append(file, fragment.start_offset()..fragment.end_offset(), false);
            previous = fragment.end_offset();
        }
        view.append(file, previous..regexp.closing.start_offset(), true);
        if shifted {
            let mut original = Self::empty(position);
            original.append(
                file,
                regexp.opening.end_offset()..regexp.closing.start_offset(),
                false,
            );
            let mut at = 0;
            let mut raw = 0;
            while at < view.bytes.len() {
                let width = char_width(&view.bytes, at);
                let raw_width = char_width(&original.bytes, raw);
                view.starts[at..at + width].fill(original.starts[raw]);
                view.ends[at..at + width].fill(original.ends[raw + raw_width - 1]);
                view.positions[at..at + width].fill(original.positions[raw]);
                at += width;
                raw += raw_width;
            }
        }
        view
    }
    fn append(&mut self, file: &SourceFile, range: Range<usize>, hole: bool) {
        let mut i = range.start;
        while i < range.end {
            let width = if file.bytes[i..range.end].starts_with(b"\r\n") {
                2
            } else {
                file.composed_character_chunk(&file.bytes[i..range.end])
                    .0
                    .max(1)
            };
            let end = (i + width).min(range.end);
            let projected = file.match_pattern_source(&file.bytes[i..end]);
            let normalized = if projected.as_ref() == b"\r\n" {
                b"\n".as_slice()
            } else {
                projected.as_ref()
            };
            let mut at = 0;
            while at < normalized.len() {
                let width = char_width(normalized, at);
                if hole {
                    self.push(b' ', i, end, true);
                } else {
                    for &b in &normalized[at..at + width] {
                        self.push(b, i, end, false);
                    }
                }
                if normalized[at] == b'\n' {
                    self.position = (self.position.0 + 1, 1);
                } else {
                    self.position.1 += 1;
                }
                at += width;
            }
            i = end;
        }
    }
    fn push(&mut self, b: u8, start: usize, end: usize, hole: bool) {
        self.bytes.push(b);
        self.starts.push(start);
        self.ends.push(end);
        self.positions.push(self.position);
        self.fingerprints.push(
            self.fingerprints
                .last()
                .unwrap()
                .wrapping_mul(257)
                .wrapping_add(u64::from(b) + 1),
        );
        self.powers
            .push(self.powers.last().unwrap().wrapping_mul(257));
        self.holes
            .push(self.holes.last().unwrap() + usize::from(hole));
    }
    fn opaque(&self, range: &Range<usize>) -> bool {
        self.holes[range.end] != self.holes[range.start]
    }
}
struct Set {
    start: usize,
    elements: Vec<Range<usize>>,
    left: Option<Range<usize>>,
    intersection: bool,
    can_start_range: bool,
    mixed_ranges: Vec<Range<usize>>,
}
#[derive(Clone, Copy)]
enum ReferenceKind {
    Backreference,
    Call,
    Condition,
}
// Narrow additional consumer: MixedRegexpCaptureTypes needs the same pinned
// source parser's successful-tree fact and capture kinds, not Ruby's effective
// capture count (which discards numbered groups when named groups exist).
pub(super) fn mixed_captures(
    regexp: &crate::cops::style::regexp_literal::Regexp<'_>,
    file: &SourceFile,
) -> bool {
    if regexp.closing.as_slice()[1..]
        .iter()
        .any(|b| matches!(b, b'e' | b's'))
    {
        return false;
    }
    let view = View::new(regexp, file);
    let mut scanner = Scanner::new(&view, regexp.closing.as_slice().contains(&b'x'));
    scanner.capture_kinds = true;
    scanner.scan();
    scanner.valid && scanner.numbered && !scanner.names.is_empty()
}
// Additional, isolated consumer of the successful source tree. Older duplicate
// set and mixed-capture behavior is unchanged. Keep raw ranges and reference
// buffer columns together, including interpolation and legacy encodings.
pub(super) fn unescaped_brackets(
    regexp: Option<&crate::cops::style::regexp_literal::Regexp<'_>>,
    content: Range<usize>,
    file: &SourceFile,
) -> Vec<(Range<usize>, (u32, u32))> {
    if regexp.is_some_and(|r| {
        r.closing.as_slice()[1..]
            .iter()
            .any(|b| matches!(b, b'e' | b's'))
    }) {
        return vec![];
    }
    let view = if let Some(regexp) = regexp {
        View::new(regexp, file)
    } else {
        let mut view = View::empty(super::duplicate_match_pattern::buffer_line_col(
            content.start,
            file,
        ));
        view.append(file, content, false);
        view
    };
    let mut scanner = Scanner::new(
        &view,
        regexp.is_some_and(|r| r.closing.as_slice().contains(&b'x')),
    );
    scanner.capture_kinds = true;
    scanner.brackets = Some(vec![]);
    scanner.scan();
    if !scanner.valid {
        return vec![];
    }
    scanner
        .brackets
        .unwrap()
        .into_iter()
        .map(|i| (view.starts[i]..view.ends[i], view.positions[i]))
        .collect()
}
// RedundantRegexpQuantifiers needs the same projected source and successful-tree prerequisite. This
// isolated entry point does not alter tokenization for existing consumers.
pub(super) fn quantifier_view(
    regexp: &crate::cops::style::regexp_literal::Regexp<'_>,
    file: &SourceFile,
) -> Option<View> {
    if regexp.closing.as_slice()[1..]
        .iter()
        .any(|b| matches!(b, b'e' | b's'))
    {
        return None;
    }
    let view = View::new(regexp, file);
    let mut scanner = Scanner::new(&view, regexp.closing.as_slice().contains(&b'x'));
    scanner.capture_kinds = true;
    scanner.brackets = Some(vec![]);
    scanner.scan();
    scanner.valid.then_some(view)
}

// MixedCaseRange consumes only actual literal ASCII bounds, never decoded escapes.
// Reuse the successful-tree/projection contract without changing older modes.
pub(super) fn mixed_case_ranges(
    regexp: &crate::cops::style::regexp_literal::Regexp<'_>,
    file: &SourceFile,
) -> Vec<(Range<usize>, (u32, u32))> {
    if regexp.closing.as_slice()[1..]
        .iter()
        .any(|b| matches!(b, b'e' | b's'))
    {
        return vec![];
    }
    let view = View::new(regexp, file);
    let mut scanner = Scanner::new(&view, regexp.closing.as_slice().contains(&b'x'));
    scanner.capture_kinds = true;
    scanner.brackets = Some(vec![]);
    scanner.mixed_ranges = Some(vec![]);
    scanner.scan();
    if !scanner.valid {
        return vec![];
    }
    scanner
        .mixed_ranges
        .unwrap()
        .into_iter()
        .map(|r| {
            let (line, col) = view.positions[r.start];
            let col = col + u32::from(line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf"));
            (view.starts[r.start]..view.ends[r.end - 1], (line, col))
        })
        .collect()
}

struct Scanner<'a> {
    view: &'a View,
    sets: Vec<Set>,
    modes: Vec<bool>,
    // Capture/bracket tree consumers follow regexp_parser's isolated-option
    // lifetime. Preserve the older duplicate-set consumer unchanged.
    capture_kinds: bool,
    brackets: Option<Vec<usize>>,
    mixed_ranges: Option<Vec<Range<usize>>>,
    option_scopes: HashSet<usize>,
    extended: bool,
    repeated: Vec<Range<usize>>,
    captures: usize,
    numbered: bool,
    // Group::Named inherits Capture but overrides #identifier with its name.
    // Only the new capture-tree consumer uses these numeric resolver targets.
    plain_capture_numbers: HashSet<usize>,
    valid: bool,
    names: HashSet<Vec<u8>>,
    references: Vec<(Vec<u8>, usize, ReferenceKind)>,
}
impl<'a> Scanner<'a> {
    fn new(view: &'a View, extended: bool) -> Self {
        Self {
            view,
            sets: vec![],
            modes: vec![],
            capture_kinds: false,
            brackets: None,
            mixed_ranges: None,
            option_scopes: HashSet::new(),
            extended,
            repeated: vec![],
            captures: 0,
            numbered: false,
            plain_capture_numbers: HashSet::new(),
            valid: false,
            names: HashSet::new(),
            references: vec![],
        }
    }
    fn duplicates(&mut self, elements: &[Range<usize>]) {
        if elements.len() < 2 {
            return;
        }
        // O(1) substring fingerprints avoid rehashing every nested set's whole
        // source. Exact byte comparison resolves collisions; hashes never decide
        // semantic equality. Each direct element is considered once.
        let mut seen: HashMap<u64, Vec<Range<usize>>> = HashMap::new();
        for range in elements {
            if self.view.opaque(range) {
                continue;
            }
            let hash = self.view.fingerprints[range.end].wrapping_sub(
                self.view.fingerprints[range.start].wrapping_mul(self.view.powers[range.len()]),
            );
            let bucket = seen.entry(hash).or_default();
            if bucket
                .iter()
                .any(|other| self.view.bytes[other.clone()] == self.view.bytes[range.clone()])
            {
                self.repeated.push(range.clone());
            } else {
                bucket.push(range.clone());
            }
        }
    }
    fn element(&mut self, mut range: Range<usize>, can_start_range: bool) {
        let Some(set) = self.sets.last_mut() else {
            return;
        };
        if let Some(left) = set.left.take() {
            // regexp_parser nests chained ranges: in [A-z-a] the direct
            // range's opening bound is itself a range, not a literal.
            if self.mixed_ranges.is_some() && set.mixed_ranges.last() == Some(&left) {
                set.mixed_ranges.pop();
            }
            if self.mixed_ranges.is_some()
                && left.len() == 1
                && range.len() == 1
                && !self.view.opaque(&(left.start..range.end))
            {
                let a = self.view.bytes[left.start];
                let b = self.view.bytes[range.start];
                if a.is_ascii_alphabetic()
                    && b.is_ascii_alphabetic()
                    && a.is_ascii_lowercase() != b.is_ascii_lowercase()
                {
                    self.sets
                        .last_mut()
                        .unwrap()
                        .mixed_ranges
                        .push(left.start..range.end);
                }
            }
            self.duplicates(&[left.clone(), range.clone()]);
            range.start = left.start;
        }
        let set = self.sets.last_mut().unwrap();
        set.elements.push(range);
        set.can_start_range = can_start_range;
    }
    // Explicit stacks, one forward token pass. Ranges and nested classes remain
    // elements in their own right; intersections are not flattened into sets.
    fn scan(&mut self) -> Vec<Range<usize>> {
        let bytes = &self.view.bytes;
        let mut i = 0;
        let mut target = false;
        let mut skip_class_closer = false;
        let mut conditional_branches: HashMap<usize, u8> = HashMap::new();
        let mut missing_reference_delimiters = [false; 2];
        while i < bytes.len() {
            let start = i;
            let b = bytes[i];
            if b == b'\\' {
                if self.sets.is_empty() && matches!(bytes.get(i + 1), Some(b'1'..=b'9')) {
                    let mut end = i + 2;
                    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                    let digits = &bytes[i + 1..end];
                    // Scanner counts only plain groups for multi-digit escape
                    // tokenization; Parser counts ALL groups for positions and
                    // relative offsets. Preserve the old consumer's threshold.
                    let opened = if self.capture_kinds {
                        self.plain_capture_numbers.len()
                    } else {
                        self.captures
                    };
                    if digits.len() == 1
                        || std::str::from_utf8(digits)
                            .unwrap()
                            .parse::<usize>()
                            .is_ok_and(|value| value <= opened)
                    {
                        self.references.push((
                            digits.to_vec(),
                            self.captures,
                            ReferenceKind::Backreference,
                        ));
                    }
                    target = true;
                    i = end;
                    continue;
                }
                // The pinned scanner recognizes numeric-looking k/g payloads
                // only as -?digits([+-]digits)?. Otherwise just \\k/\\g is a
                // literal escape and the following text is scanned normally.
                if self.capture_kinds
                    && self.sets.is_empty()
                    && matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                    && !capture_reference_token(bytes, i, &mut missing_reference_delimiters)
                {
                    target = true;
                    i += 2;
                    continue;
                }
                let end = if self.brackets.is_some()
                    && matches!(bytes.get(i + 1), Some(b'c' | b'C' | b'M'))
                {
                    bracket_control_escape_end(bytes, i)
                } else {
                    escape_end(bytes, i, !self.sets.is_empty())
                };
                let Some(end) = end else {
                    return vec![];
                };
                if self.sets.is_empty()
                    && matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                    && end > i + 3
                {
                    let mut reference = &bytes[i + 3..end - 1];
                    if !self.capture_kinds
                        && bytes[i + 1] == b'k'
                        && let Some(suffix) = reference
                            .iter()
                            .enumerate()
                            .skip(1)
                            .find_map(|(i, b)| b"+-".contains(b).then_some(i))
                    {
                        reference = &reference[..suffix];
                    }
                    let kind = if bytes[i + 1] == b'g' {
                        ReferenceKind::Call
                    } else {
                        ReferenceKind::Backreference
                    };
                    self.references
                        .push((reference.to_vec(), self.captures, kind));
                }
                self.element(i..end, true);
                target = true;
                i = end;
                continue;
            }
            if !self.sets.is_empty() {
                if b == b']' {
                    let set = self.sets.pop().unwrap();
                    if self.brackets.is_some() && set.elements.is_empty() && !set.intersection {
                        skip_class_closer = true;
                    }
                    if set.left.is_some() {
                        return vec![];
                    }
                    if !set.intersection {
                        self.duplicates(&set.elements);
                        if let Some(ranges) = &mut self.mixed_ranges {
                            ranges.extend(set.mixed_ranges);
                        }
                    }
                    // A subset's closing token is :set, never a range start.
                    self.element(set.start..i + 1, false);
                    target = true;
                    i += 1;
                    continue;
                }
                if bytes[i..].starts_with(b"&&") {
                    let set = self.sets.last_mut().unwrap();
                    set.intersection = true;
                    set.elements.clear();
                    set.can_start_range = false;
                    i += 2;
                    continue;
                }
                if b == b'-'
                    && bytes.get(i + 1).is_some_and(|b| *b != b']')
                    && !bytes[i + 1..].starts_with(b"&&")
                {
                    let set = self.sets.last_mut().unwrap();
                    if set.left.is_none() && set.can_start_range {
                        set.left = set.elements.pop();
                        i += 1;
                        continue;
                    }
                }
                if bytes[i..].starts_with(b"[:") {
                    let Some(end) = bytes[i + 2..]
                        .windows(2)
                        .position(|s| s == b":]")
                        .map(|end| i + end + 4)
                    else {
                        return vec![];
                    };
                    let name = bytes[i + 2..end - 2]
                        .strip_prefix(b"^")
                        .unwrap_or(&bytes[i + 2..end - 2]);
                    if ![
                        b"alnum".as_slice(),
                        b"alpha",
                        b"ascii",
                        b"blank",
                        b"cntrl",
                        b"digit",
                        b"graph",
                        b"lower",
                        b"print",
                        b"punct",
                        b"space",
                        b"upper",
                        b"word",
                        b"xdigit",
                    ]
                    .contains(&name)
                    {
                        return vec![];
                    }
                    self.element(i..end, true);
                    i = end;
                    continue;
                }
            } else {
                if b == b']'
                    && let Some(brackets) = &mut self.brackets
                {
                    if skip_class_closer {
                        skip_class_closer = false;
                    } else if i != 0 {
                        brackets.push(i);
                    }
                }
                if b.is_ascii_whitespace() && self.extended {
                    i += 1;
                    continue;
                }
                if matches!(b, b'*' | b'+' | b'?') || b == b'{' && interval_end(bytes, i).is_some()
                {
                    if !target {
                        return vec![];
                    }
                    i = interval_end(bytes, i).unwrap_or(i + 1);
                    continue;
                }
                if b == b'|' {
                    if let Some(count) = conditional_branches.get_mut(&self.modes.len()) {
                        *count += 1;
                        if *count > 2 {
                            return vec![];
                        }
                    }
                    target = false;
                    i += 1;
                    continue;
                }
                if b == b'#' && self.extended {
                    i += bytes[i..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .unwrap_or(bytes.len() - i);
                    continue;
                }
                if bytes[i..].starts_with(b"(?#") {
                    i += 3;
                    // regexp_parser closes even at an escaped ')'. Ruby may
                    // accept more, but a leftover unmatched ')' makes its tree nil.
                    while i < bytes.len() && bytes[i] != b')' {
                        i += char_width(bytes, i);
                    }
                    if i == bytes.len() {
                        return vec![];
                    }
                    i += 1;
                    continue;
                }
                if b == b'(' {
                    target = false;
                    let old = self.extended;
                    if bytes.get(i + 1) == Some(&b'?') {
                        if bytes.get(i + 2) == Some(&b'(') {
                            let Some(end) = bytes[i + 3..]
                                .iter()
                                .position(|b| *b == b')')
                                .map(|end| i + 3 + end)
                            else {
                                return vec![];
                            };
                            let mut reference = &bytes[i + 3..end];
                            if self.capture_kinds && !capture_condition_token(reference) {
                                return vec![];
                            }
                            if matches!(reference.first(), Some(b'<' | b'\''))
                                && reference.len() >= 2
                            {
                                reference = &reference[1..reference.len() - 1];
                            }
                            self.references.push((
                                reference.to_vec(),
                                self.captures,
                                ReferenceKind::Condition,
                            ));
                            self.modes.push(old);
                            conditional_branches.insert(self.modes.len(), 1);
                            i = end + 1;
                            continue;
                        }
                        let mut end = i + 2;
                        let mut enabled = true;
                        while end < bytes.len() && b"imxadu-".contains(&bytes[end]) {
                            match bytes[end] {
                                b'-' => enabled = false,
                                b'x' => self.extended = enabled,
                                // Pinned scanner rejects charset flags after
                                // any '-': only i/m/x may be disabled. Keep
                                // successful-tree validation capture-only.
                                b'a' | b'd' | b'u' if self.capture_kinds && !enabled => {
                                    return vec![];
                                }
                                _ => {}
                            }
                            end += 1;
                        }
                        if end > i + 2 && bytes.get(end) == Some(&b')') {
                            i = end + 1;
                            continue;
                        }
                        if end > i + 2 && bytes.get(end) == Some(&b':') {
                            self.modes.push(old);
                            if self.capture_kinds {
                                self.option_scopes.insert(self.modes.len());
                            }
                            i = end + 1;
                            continue;
                        }
                        self.extended = old;
                        if !matches!(
                            bytes.get(i + 2),
                            Some(b':' | b'=' | b'!' | b'>' | b'~' | b'<' | b'\'')
                        ) {
                            return vec![];
                        }
                        let name_end = match bytes.get(i + 2) {
                            Some(b'<') if !matches!(bytes.get(i + 3), Some(b'=' | b'!')) => {
                                Some(b'>')
                            }
                            Some(b'\'') => Some(b'\''),
                            _ => None,
                        };
                        if let Some(delimiter) = name_end {
                            let Some(end) = bytes[i + 3..].iter().position(|b| *b == delimiter)
                            else {
                                return vec![];
                            };
                            if end == 0 {
                                return vec![];
                            }
                            let name = &bytes[i + 3..i + 3 + end];
                            // Pinned scanner's named-group grammar excludes a
                            // leading ASCII digit or minus, not arbitrary Ruby
                            // identifier punctuation. Preserve the older cop.
                            if self.capture_kinds && (name[0].is_ascii_digit() || name[0] == b'-') {
                                return vec![];
                            }
                            self.names.insert(name.to_vec());
                            self.captures += 1;
                            i += end + 4;
                            self.modes.push(old);
                            continue;
                        }
                    }
                    if bytes.get(i + 1) != Some(&b'?') {
                        self.numbered = true;
                        self.captures += 1;
                        if self.capture_kinds {
                            self.plain_capture_numbers.insert(self.captures);
                        }
                    }
                    self.modes.push(old);
                    i += if bytes.get(i + 1) == Some(&b'?') {
                        if bytes.get(i + 2) == Some(&b'<') {
                            4
                        } else {
                            3
                        }
                    } else {
                        1
                    };
                    continue;
                } else if b == b')' {
                    conditional_branches.remove(&self.modes.len());
                    target = true;
                    let Some(old) = self.modes.pop() else {
                        return vec![];
                    };
                    if !self.capture_kinds || self.option_scopes.remove(&(self.modes.len() + 1)) {
                        self.extended = old;
                    }
                } else {
                    target = true;
                }
            }
            if b == b'[' {
                self.sets.push(Set {
                    start,
                    elements: vec![],
                    left: None,
                    intersection: false,
                    can_start_range: false,
                    mixed_ranges: vec![],
                });
                i += 1;
                if bytes.get(i) == Some(&b'^') {
                    i += 1;
                }
                continue;
            }
            i += char_width(bytes, i);
            self.element(start..i, true);
        }
        if !self.sets.is_empty() || !self.modes.is_empty() {
            return vec![];
        }
        for (reference, at, kind) in &self.references {
            if self.capture_kinds {
                if !self.capture_reference_valid(reference, *at, *kind) {
                    return vec![];
                }
                continue;
            }
            if let Ok(value) = std::str::from_utf8(reference).unwrap_or("").parse::<i64>() {
                let relative = matches!(reference.first(), Some(b'-' | b'+'));
                if value == 0 && (!matches!(kind, ReferenceKind::Call) || reference != b"0")
                    || matches!(kind, ReferenceKind::Backreference) && reference.starts_with(b"+")
                {
                    return vec![];
                }
                let target = if reference.starts_with(b"-") {
                    *at as i64 + value + 1
                } else if reference.starts_with(b"+") {
                    *at as i64 + value
                } else {
                    value
                };
                if target < i64::from(relative) || target > self.captures as i64 {
                    return vec![];
                }
            } else if !self.names.contains(reference) {
                return vec![];
            }
        }
        self.repeated.sort_by_key(|r| (r.start, r.end));
        self.repeated.dedup();
        self.valid = true;
        std::mem::take(&mut self.repeated)
    }

    // Port of the pinned scanner's k/g token actions, Backreference classes,
    // Parser#assign_effective_number and Base#assign_referenced_expressions.
    // Used by capture/bracket tree consumers: old set detection/fixes are intact.
    fn capture_reference_valid(&self, reference: &[u8], at: usize, kind: ReferenceKind) -> bool {
        let named = reference.first().is_some_and(|b| {
            !b.is_ascii_digit()
                && *b != b'-'
                && (!matches!(kind, ReferenceKind::Call) || *b != b'+')
        });
        if named {
            if matches!(kind, ReferenceKind::Backreference) {
                let suffix = reference
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find_map(|(i, b)| b"+-".contains(b).then_some(i));
                if let Some(suffix) = suffix {
                    // NameRecursionLevel splits at the FIRST sign, but the
                    // scanner requires the token to END with sign + digits.
                    let last = reference.iter().rposition(|b| b"+-".contains(b)).unwrap();
                    return !reference[1..].contains(&b'\n')
                        && ascii_digits(&reference[last + 1..])
                        && self.names.contains(&reference[..suffix]);
                }
            }
            // The scanner's name-call action uses `.*` (not multiline).
            return (!matches!(kind, ReferenceKind::Call) || !reference[1..].contains(&b'\n'))
                && self.names.contains(reference);
        }
        let signed = matches!(reference.first(), Some(b'-' | b'+'));
        let digits_start = usize::from(signed);
        let digits_end = digits_start
            + reference[digits_start..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
        if digits_end == digits_start {
            return false;
        }
        let Ok(value) = std::str::from_utf8(&reference[..digits_end])
            .unwrap_or("")
            .parse::<i64>()
        else {
            return false;
        };
        let suffix = &reference[digits_end..];
        if matches!(kind, ReferenceKind::Backreference)
            && !suffix.is_empty()
            && !(b"+-".contains(&suffix[0]) && ascii_digits(&suffix[1..]))
        {
            return false;
        }
        if matches!(kind, ReferenceKind::Call) && !signed && !suffix.is_empty() {
            return false;
        }
        if value == 0 {
            return matches!(kind, ReferenceKind::Call) && reference == b"0";
        }
        let target = if signed {
            // All captures count for relative offsets, including named ones.
            let Some(target) = (at as i64)
                .checked_add(value)
                .and_then(|n| n.checked_add(i64::from(value < 0)))
            else {
                return false;
            };
            target
        } else {
            value
        };
        target > 0 && self.plain_capture_numbers.contains(&(target as usize))
    }
}

fn ascii_digits(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

pub(super) fn capture_reference_token(
    bytes: &[u8],
    start: usize,
    missing_delimiters: &mut [bool; 2],
) -> bool {
    let Some(&opening) = bytes.get(start + 2).filter(|b| matches!(b, b'<' | b'\'')) else {
        return false;
    };
    let close = if opening == b'<' { b'>' } else { b'\'' };
    let Some(&first) = bytes.get(start + 3) else {
        return false;
    };
    // Empty delimited payloads reach the scanner's validation error. Name
    // payloads consume through the delimiter. For numeric-looking payloads,
    // stop at the first non-grammar byte instead of rescanning an arbitrary
    // suffix on every literal \\k/\\g fallback.
    if first == close {
        return true;
    }
    if !first.is_ascii_digit() && first != b'-' {
        let missing = &mut missing_delimiters[usize::from(close == b'>')];
        if !*missing && bytes[start + 3..].contains(&close) {
            return true;
        }
        *missing = true;
        return false;
    }
    let mut end = start + 3 + usize::from(first == b'-');
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == digits_start {
        return false;
    }
    if matches!(bytes.get(end), Some(b'+' | b'-')) {
        end += 1;
        let suffix_start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == suffix_start {
            return false;
        }
    }
    bytes.get(end) == Some(&close)
}

fn capture_condition_token(reference: &[u8]) -> bool {
    if matches!(reference.first(), Some(b'<' | b'\'')) {
        let close = if reference[0] == b'<' { b'>' } else { b'\'' };
        reference.len() >= 3
            && reference.last() == Some(&close)
            && !reference[1].is_ascii_digit()
            && reference[1] != b'-'
    } else {
        ascii_digits(reference) && reference.iter().any(|b| *b != b'0')
    }
}

pub(super) fn interval_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut i = start + 1;
    let mut digits = 0;
    let mut comma = false;
    while let Some(b) = bytes.get(i) {
        if b.is_ascii_digit() {
            digits += 1;
        } else if *b == b',' && !comma {
            comma = true;
        } else {
            return (*b == b'}' && digits > 0).then_some(i + 1);
        }
        i += 1;
    }
    None
}
pub(super) fn char_width(bytes: &[u8], i: usize) -> usize {
    bytes
        .get(i)
        .map_or(0, |b| match b {
            0xc2..=0xdf if bytes.get(i + 1).is_some_and(|b| b & 0xc0 == 0x80) => 2,
            0xe0..=0xef
                if bytes
                    .get(i..i + 3)
                    .is_some_and(|s| std::str::from_utf8(s).is_ok()) =>
            {
                3
            }
            0xf0..=0xf4
                if bytes
                    .get(i..i + 4)
                    .is_some_and(|s| std::str::from_utf8(s).is_ok()) =>
            {
                4
            }
            _ => 1,
        })
        .min(bytes.len() - i)
}
// Pinned regexp_parser control/meta tokens allow a single opposite modifier,
// then an optionally backslashed ASCII character, NOT recursive Ruby escapes.
// Thus \\c\\c ends before a following ], while bare \\C/\\M fails the tree.
// Isolate this spelling behavior to the bracket consumer: inherited duplicate-
// set/mixed-capture semantics are deliberately unchanged.
pub(super) fn bracket_control_escape_end(bytes: &[u8], start: usize) -> Option<usize> {
    let kind = *bytes.get(start + 1)?;
    let mut end = start + 2;
    if kind != b'c' {
        if bytes.get(end) != Some(&b'-') {
            return None;
        }
        end += 1;
    }
    if kind == b'M' {
        if bytes[end..].starts_with(b"\\c") {
            end += 2;
        } else if bytes[end..].starts_with(b"\\C-") {
            end += 3;
        }
    } else if bytes[end..].starts_with(b"\\M-") {
        end += 3;
    }
    if bytes.get(end) == Some(&b'\\') {
        end += 1;
    }
    bytes.get(end).filter(|b| b.is_ascii()).map(|_| end + 1)
}

pub(super) fn escape_end(bytes: &[u8], mut i: usize, in_set: bool) -> Option<usize> {
    loop {
        let kind = *bytes.get(i + 1)?;
        let next = match kind {
            b'c' => Some(i + 2),
            b'C' | b'M' if bytes.get(i + 2) == Some(&b'-') => Some(i + 3),
            _ => None,
        };
        if let Some(next) = next {
            if bytes.get(next) == Some(&b'\\') {
                i = next;
                continue;
            }
            return bytes.get(next).map(|_| next + char_width(bytes, next));
        }
        let mut end = i + 1 + char_width(bytes, i + 1);
        match kind {
            b'u' | b'p' | b'P' if bytes.get(end) == Some(&b'{') => {
                end += 1;
                end += bytes[end..].iter().position(|b| *b == b'}')? + 1;
                let payload = &bytes[i + 3..end - 1];
                if kind == b'u' {
                    // Pinned grammar: 1..6 hex digits per item, separated by
                    // exactly one ASCII whitespace byte; no surrounding space.
                    let mut digits = 0;
                    for &b in payload {
                        if b.is_ascii_hexdigit() {
                            digits += 1;
                            if digits > 6 {
                                return None;
                            }
                        } else if b" \t\n\r\x0b\x0c".contains(&b) && digits > 0 {
                            digits = 0;
                        } else {
                            return None;
                        }
                    }
                    if digits == 0 {
                        return None;
                    }
                } else if !known_property(payload) {
                    return None;
                }
            }
            b'k' | b'g' if !in_set && matches!(bytes.get(end), Some(b'<' | b'\'')) => {
                let close = if bytes[end] == b'<' { b'>' } else { b'\'' };
                end += 1;
                end += bytes[end..].iter().position(|b| *b == close)? + 1;
            }
            b'u' | b'x' | b'0'..=b'7' => {
                let max = match kind {
                    b'u' => 4,
                    b'x' => 2,
                    _ => 2,
                };
                for _ in 0..max {
                    if bytes.get(end).is_some_and(|b| {
                        if kind.is_ascii_digit() {
                            matches!(b, b'0'..=b'7')
                        } else {
                            b.is_ascii_hexdigit()
                        }
                    }) {
                        end += 1;
                    } else {
                        break;
                    }
                }
                if kind == b'u' && end != i + 6 || kind == b'x' && end == i + 2 {
                    return None;
                }
            }
            _ => {}
        }
        return Some(end);
    }
}

// Pinned regexp_parser 2.13.1 property aliases (MIT, Copyright (c) 2010-2026,
// Ammar Ali; permission and warranty terms in LICENSE). Regeneration/hash checks live in
// tools/oracle/test/duplicate_regexp_character_class_element_test.rb.
fn known_property(name: &[u8]) -> bool {
    static NAMES: std::sync::OnceLock<Vec<&str>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| {
        include_str!("duplicate_regexp_properties.txt")
            .lines()
            .collect()
    });
    let normalized: Vec<u8> = name
        .iter()
        .filter(|b| !b.is_ascii_whitespace() && !b"^_-".contains(b))
        .map(u8::to_ascii_lowercase)
        .collect();
    std::str::from_utf8(&normalized).is_ok_and(|name| names.binary_search(&name).is_ok())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/duplicate_regexp_character_class_element_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateRegexpCharacterClassElement")
            .collect()
    }
    #[test]
    fn pinned_character_range_escape_posix_and_interpolation_examples() {
        for (source, cols) in [
            ("foo = /[xyx]/", vec![11]),
            ("foo = /[xyx]+/", vec![11]),
            ("foo = %r{[xyx]}", vec![13]),
            ("foo = /([xyx])/", vec![12]),
            ("foo = /([[:alnum:]y[:alnum:]])/", vec![20]),
            (r"/[\0\08]/", vec![5]),
            (r"/[\07\01\078]/", vec![9]),
            (r"/[\177\01\1778]/", vec![10]),
            ("foo = /([a#{foo}a#{bar}a])/", vec![17, 24]),
            ("foo = /[0-9x0-9]/", vec![13]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line))
                    .collect::<Vec<_>>(),
                cols.into_iter().map(|c| (1, c, 1)).collect::<Vec<_>>(),
                "{source}"
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Duplicate element inside regexp character class")
            );
        }
    }
    #[test]
    fn all_pinned_and_control_fixtures_exact_detection_and_correction() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_regexp_character_class_element.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            if row["invalid_syntax"] != true && !expected.is_empty() {
                crate::cops::testing::assert_correction(
                    &super::DuplicateRegexpCharacterClassElement,
                    source,
                    row["project_after"].as_str().unwrap(),
                );
            }
        }
    }
    #[test]
    fn wide_and_deep_patterns_use_iterative_bounded_source_work() {
        let source = format!("/[{}]/", "a".repeat(12000));
        assert_eq!(offenses(&source).len(), 11999);
        // Deep regexp nesting is not Ruby AST nesting. Exercise the scanner
        // directly, never ask a native regexp compiler to consume this input.
        let bytes = format!("{}z{}", "[ab".repeat(12000), "]".repeat(12000));
        let mut view = super::View::empty((1, 1));
        for (i, b) in bytes.bytes().enumerate() {
            view.push(b, i, i + 1, false);
        }
        assert!(super::Scanner::new(&view, false).scan().is_empty());
        let opaque = format!("/[{}a]/", "#{value}".repeat(6000));
        assert!(offenses(&opaque).is_empty());
    }
    #[test]
    fn nested_sets_do_not_start_ranges() {
        for (source, cols) in [("/[[ab]-bb]/", vec![9]), ("/[&&[a-a]-[a-a]]/", vec![8, 14])] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                cols.into_iter().map(|col| (1, col)).collect::<Vec<_>>(),
                "{source}"
            );
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Duplicate element inside regexp character class")
            );
        }
    }
    #[test]
    fn numeric_escape_fallback_uses_open_capture_count() {
        for (source, cols) in [
            (r"/[aa]\99/", vec![4]),
            (r"/[aa]#{value}\12345/", vec![4]),
            (r"/[aa]#{value}\99999999999999999999999/", vec![4]),
            (r"/(a)[aa]#{value}\11/", vec![7]),
            (r"/[aa]#{value}\1/", vec![]),
            (r"/\11(a)(a)(a)(a)(a)(a)(a)(a)(a)(a)(a)[aa]/", vec![40]),
        ] {
            assert_eq!(
                offenses(source).iter().map(|o| o.col).collect::<Vec<_>>(),
                cols,
                "{source}"
            );
        }
    }
    #[test]
    fn zero_backreferences_are_not_whole_regexp_calls() {
        for source in [
            r"/[aa]#{value}\k<0>/",
            r"/[aa]#{value}\k'0'/",
            r"/[aa]#{value}\g<00>/",
            r"/[aa]#{value}\g<+0>/",
            r"/[aa]#{value}\k<-1>/",
            r"/[aa]#{value}\g<-1>/",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses(r"/[aa]#{value}\g<0>/")[0].col, 4);
    }
    #[test]
    fn braced_unicode_payload_must_match_pinned_grammar() {
        for payload in ["", "zzzz", " 61", "61 ", "61  62", "1234567"] {
            let source = format!(r"/[aa]#{{value}}\u{{{payload}}}/");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for payload in [
            "61",
            "61 62",
            "61\t62",
            "61\n62",
            "61\u{b}62",
            "10ffff",
            "000061",
        ] {
            let source = format!("/[aa]#{{value}}\\u{{{payload}}}/");
            assert_eq!(
                offenses(&source).iter().map(|o| o.col).collect::<Vec<_>>(),
                vec![4],
                "{source}"
            );
        }
    }
    #[test]
    fn group_comments_close_at_first_parenthesis() {
        assert!(offenses(r"/(?#x\))[aa]/").is_empty());
        assert_eq!(offenses(r"/(?#x)[aa]/")[0].col, 9);
    }
    #[test]
    fn negated_set_closes_at_leading_closing_bracket() {
        assert!(offenses("/[^]aa]/").is_empty());
        assert_eq!(offenses("/[^aa]/")[0].col, 5);
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "foo = /[xyz]/",
            "foo = /[xyz][xyz]/",
            r"/[\0\07]/",
            "foo = /[ab&&ab]/",
            "foo = /[a-cb]/",
            "a.gsub!(/[#{bad_chars}]/, '')\nb.gsub!(/[#{bad_chars}]/, '')",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
