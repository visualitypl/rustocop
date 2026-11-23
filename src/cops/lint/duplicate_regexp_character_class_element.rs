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
        let mut scanner = Scanner::new(&view, regexp.closing.as_slice().contains(&b'x'));
        // All source-tree consumers share the pinned token/reference grammar.
        // The capture validation already handles named capture identifiers, literal
        // reference fallback and invalid option/name headers without reparsing.
        let ranges = scanner.scan();
        // RuboCop removes each repeated element.
        let correctable = cx.corrections_requested();
        for range in ranges {
            let raw = view.starts[range.start]..view.ends[range.end - 1];
            let (line, col) = view.positions[range.start];
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Duplicate element inside regexp character class".into(),
                line,
                col,
                // Parser::Source::Range#last_line includes a token's terminal
                // physical LF (e.g. a repeated raw control-newline escape).
                end_line: file.line_of(raw.end),
                correction: correctable.then_some(Correction {
                    range: raw,
                    replacement: String::new(),
                }),
            });
        }
    }
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
        let mut interpolation_ranges = vec![];
        for (fragment, &decoded_lines) in regexp.fragments.iter().zip(&regexp.fragment_line_counts)
        {
            let shorthand = file.bytes.get(previous) == Some(&b'#')
                && matches!(file.bytes.get(previous + 1), Some(b'@' | b'$'));
            // Upstream only blanks begin nodes, not #@ivar/#$gvar/#@@cvar.
            // Those children expose source WITHOUT '#'; retain that exact
            // raw-buffer quirk, including the consequent projected locations.
            shifted |= shorthand;
            if !shorthand && previous < fragment.start_offset() {
                interpolation_ranges.push(previous..fragment.start_offset());
            }
            view.position.1 += u32::from(shorthand);
            view.append(
                file,
                previous + usize::from(shorthand)..fragment.start_offset(),
                !shorthand,
            );
            shifted |=
                view.append_fragment(file, fragment, decoded_lines, regexp.opening.as_slice());
            previous = fragment.end_offset();
        }
        view.append(file, previous..regexp.closing.start_offset(), true);
        if previous < regexp.closing.start_offset() {
            interpolation_ranges.push(previous..regexp.closing.start_offset());
        }
        if shifted {
            let mut original = Self::empty(position);
            let mut previous = regexp.opening.end_offset();
            for range in interpolation_ranges {
                original.append(file, previous..range.start, false);
                original.append(file, range.clone(), true);
                previous = range.end;
            }
            original.append(file, previous..regexp.closing.start_offset(), false);
            // Upstream overlap tests use ORIGINAL interpolation source ranges,
            // even when projected str/shorthand text shifts later expressions.
            view.holes = vec![0];
            let mut at = 0;
            let mut raw = 0;
            while at < view.bytes.len() {
                let width = char_width(&view.bytes, at);
                let raw_width = char_width(&original.bytes, raw);
                view.starts[at..at + width].fill(original.starts[raw]);
                view.ends[at..at + width].fill(original.ends[raw + raw_width - 1]);
                view.positions[at..at + width].fill(original.positions[raw]);
                let opaque = original.opaque(&(raw..raw + raw_width));
                for _ in 0..width {
                    view.holes
                        .push(view.holes.last().unwrap() + usize::from(opaque));
                }
                at += width;
                raw += raw_width;
            }
        }
        view
    }
    // Prism1.9's string_nodes_from_line_continuations (MIT, Shopify Inc.)
    // emits only as many grouped source lines as the MAIN parse's cooked.lines.
    // A controlled physical LF becomes text \\x0A, so trailing raw lines may
    // have no emitted str range. Project that exact prefix, not the regexp value;
    // never blank/drop an arbitrary detector or parse/decode a candidate.
    fn append_fragment(
        &mut self,
        file: &SourceFile,
        fragment: &ruby_prism::Location<'_>,
        decoded_lines: usize,
        opening: &[u8],
    ) -> bool {
        let start = self.bytes.len();
        self.append(file, fragment.start_offset()..fragment.end_offset(), false);
        let bytes = &self.bytes[start..];
        if !bytes.contains(&b'\n') || opening.ends_with(b"'") {
            return false;
        }
        // Buffer CRLF normalization happens BEFORE the reference main parse.
        // A control/meta operand CRLF in our physical parse leaves a cooked LF;
        // after normalization that LF is the operand (text \\x0A), not a line.
        // Adjust only that proven difference, without decoding any escape value.
        let mut controlled_crlfs = 0;
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'\\' {
                if matches!(bytes.get(at + 1), Some(b'c' | b'C' | b'M'))
                    && let Some(end) = bracket_control_escape_end(bytes, at)
                {
                    let operand = start + end - 1;
                    controlled_crlfs += usize::from(
                        bytes[end - 1] == b'\n'
                            && file.bytes[self.starts[operand]..self.ends[operand]] == *b"\r\n",
                    );
                    at = end;
                } else {
                    at += 1 + char_width(bytes, at + 1);
                }
            } else {
                at += char_width(bytes, at);
            }
        }
        let decoded_lines = decoded_lines.saturating_sub(controlled_crlfs);
        let mut groups = 0;
        let mut consumed = 0;
        let mut emitted_end = 0;
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            consumed += line.len();
            let body = line.strip_suffix(b"\n").unwrap_or(line);
            let body = body.strip_suffix(b"\r").unwrap_or(body);
            let continued = line.ends_with(b"\n")
                && body.iter().rev().take_while(|b| **b == b'\\').count() % 2 == 1;
            if !continued || consumed == bytes.len() {
                groups += 1;
                if groups <= decoded_lines {
                    emitted_end = consumed;
                }
            }
        }
        let end = start + emitted_end;
        if end == self.bytes.len() {
            return false;
        }
        self.position = self.positions[end];
        self.bytes.truncate(end);
        self.starts.truncate(end);
        self.ends.truncate(end);
        self.positions.truncate(end);
        self.holes.truncate(end + 1);
        self.fingerprints.truncate(end + 1);
        self.powers.truncate(end + 1);
        true
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
    scanner.brackets = Some(vec![]);
    scanner.scan();
    scanner.valid.then_some(view)
}

// MixedCaseRange consumes only actual literal ASCII bounds, never decoded escapes.
// Reuse the shared successful-tree/projection contract.
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
    brackets: Option<Vec<usize>>,
    mixed_ranges: Option<Vec<Range<usize>>>,
    option_scopes: HashSet<usize>,
    extended: bool,
    repeated: Vec<Range<usize>>,
    captures: usize,
    numbered: bool,
    // Group::Named inherits Capture but overrides #identifier with its name.
    // Numeric references resolve only to these plain-capture identifiers.
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
                    // relative offsets, including named groups.
                    let opened = self.plain_capture_numbers.len();
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
                if self.sets.is_empty()
                    && matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                    && !capture_reference_token(bytes, i, &mut missing_reference_delimiters)
                {
                    target = true;
                    i += 2;
                    continue;
                }
                let Some(end) = escape_end(bytes, i, !self.sets.is_empty()) else {
                    return vec![];
                };
                if self.sets.is_empty()
                    && matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                    && end > i + 3
                {
                    let reference = &bytes[i + 3..end - 1];
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
                // regexp_parser's extended-mode space class includes VT (0B),
                // unlike Rust's ASCII whitespace predicate. Inside sets these
                // bytes remain literal elements, not ignored spacing.
                if self.extended && b" \t\n\r\x0b\x0c".contains(&b) {
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
                            let Some(end) = capture_condition_end(bytes, i + 3) else {
                                return vec![];
                            };
                            // Conditional::Condition#reference deletes ALL of
                            // these characters, even inside a permitted name.
                            let reference = bytes[i + 3..end]
                                .iter()
                                .copied()
                                .filter(|b| !b"'<>()".contains(b))
                                .collect();
                            self.references.push((
                                reference,
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
                                // any '-': only i/m/x may be disabled.
                                b'a' | b'd' | b'u' if !enabled => {
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
                            self.option_scopes.insert(self.modes.len());
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
                            // identifier punctuation.
                            if name[0].is_ascii_digit() || name[0] == b'-' {
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
                        self.plain_capture_numbers.insert(self.captures);
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
                    if self.option_scopes.remove(&(self.modes.len() + 1)) {
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
            if !self.capture_reference_valid(reference, *at, *kind) {
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
    // Shared by every source-tree consumer; no divergent legacy set resolver.
    fn capture_reference_valid(&self, reference: &[u8], at: usize, kind: ReferenceKind) -> bool {
        // A condition's negative-looking reference remains a String, NOT a
        // relative backreference. The parser resolves it as a capture name.
        if matches!(kind, ReferenceKind::Condition) && !ascii_digits(reference) {
            return self.names.contains(reference);
        }
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
            // regexp_parser uses arbitrary precision integers. An overflowing
            // target cannot name a capture in the bounded source: no wrapping.
            let Some(target) = i64::try_from(at)
                .ok()
                .and_then(|at| at.checked_add(value))
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

// Pinned condition token: a delimited name (first byte excludes digit,
// minus and its closer; remaining bytes exclude only that closer), or -?digits.
// Parentheses inside names are legal; the first matching delimiter MUST be
// immediately followed by ')'. No search through an embedded closer.
pub(super) fn capture_condition_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start)?;
    let mut end = start;
    if matches!(first, b'<' | b'\'') {
        let close = if first == b'<' { b'>' } else { b'\'' };
        let first_name = *bytes.get(start + 1)?;
        if first_name.is_ascii_digit() || first_name == b'-' || first_name == close {
            return None;
        }
        end = start + 1 + bytes[start + 1..].iter().position(|b| *b == close)? + 1;
    } else {
        end += usize::from(first == b'-');
        let digits_start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == digits_start || bytes[digits_start..end].iter().all(|b| *b == b'0') {
            return None;
        }
    }
    (bytes.get(end) == Some(&b')')).then_some(end)
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
// All source-tree consumers must use these same raw boundaries, including
// duplicate sets and successful-tree validation (not just bracket reporting).
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

pub(super) fn escape_end(bytes: &[u8], i: usize, in_set: bool) -> Option<usize> {
    let kind = *bytes.get(i + 1)?;
    if matches!(kind, b'c' | b'C' | b'M') {
        return bracket_control_escape_end(bytes, i);
    }
    let mut end = i + 1 + char_width(bytes, i + 1);
    match kind {
        b'p' | b'P' if bytes.get(end) == Some(&b'{') => {
            let mut at = end + 1;
            if bytes.get(at) == Some(&b'^') {
                at += 1;
            }
            let name_start = at;
            loop {
                let b = *bytes.get(at)?; // valid but unfinished property
                if b == b'}' {
                    if at != name_start {
                        if !known_property(&bytes[name_start..at]) {
                            return None;
                        }
                        end = at + 1;
                    }
                    break;
                }
                // The token grammar precedes alias lookup. Any other byte
                // makes ONLY \\p/\\P a literal escape; rescan the braces
                // and payload normally (including set duplicate elements).
                if !b.is_ascii_alphanumeric() && !b"_-.= \t\n\r\x0b\x0c".contains(&b) {
                    break;
                }
                at += 1;
            }
        }
        b'u' if bytes.get(end) == Some(&b'{') => {
            end += 1;
            end += bytes[end..].iter().position(|b| *b == b'}')? + 1;
            let payload = &bytes[i + 3..end - 1];
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
    Some(end)
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
        // Ruby's whitespace class includes VT (0B); is_ascii_whitespace does not.
        .filter(|b| !b"^_- \t\n\r\x0b\x0c".contains(b))
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
    fn operator_recovery_source_oracle_rows_match_exactly() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_regexp_extended.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(
                offenses(source)
                    .into_iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
        }
    }
    #[test]
    fn operator_round2_shared_source_tree_consumers_match_actual_pinned_team() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_regexp_extended.json"
        ))
        .unwrap();
        let names = [
            "Lint/DuplicateRegexpCharacterClassElement",
            "Lint/MixedRegexpCaptureTypes",
            "Lint/UnescapedBracketInRegexp",
            "Lint/RedundantRegexpQuantifiers",
            "Lint/MixedCaseRange",
        ];
        let mut checked = 0;
        for row in rows {
            let Some(expected) = row.get("shared_findings") else {
                continue;
            };
            checked += 1;
            let mut actual = lint_str(row["source"].as_str().unwrap())
                .into_iter()
                .filter(|o| names.contains(&o.cop))
                .map(|o| (o.cop.to_string(), o.line, o.col, o.end_line, o.message))
                .collect::<Vec<_>>();
            actual.sort();
            let expected: Vec<(String, u32, u32, u32, String)> = crate::fixture::value(expected);
            assert_eq!(actual, expected, "{row}");
        }
        assert_eq!(checked, 607);
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
    fn isolated_extended_switches_survive_non_option_group_closers() {
        // The pinned source scanner's
        // free-spacing switch is restored by option groups, not ordinary ones.
        for source in [
            "/(?<n>(?x))#[aa]/\n",
            "/((?x))#[aa]/\n",
            "/(?:(?x))#[aa]/\n",
            "/(?=(?x))#[aa]/\n",
            "/(?<=(?x))#[aa]/\n",
            "/(?>(?x))#[aa]/\n",
            "/(?~(?x))#[aa]/\n",
            "/(?<n>(?x))# [aa]#{value}/\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "/(?<n>(?x))# [aa]\n[bb]/",
            "/(?x:(?-x))#[aa]/",
            "/(?<n>(?-x))#[aa]/x",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let offset = source.rfind("bb").or_else(|| source.rfind("aa")).unwrap() + 1;
            let prefix = &source[..offset];
            assert_eq!(
                found[0].line,
                prefix.bytes().filter(|b| *b == b'\n').count() as u32 + 1
            );
            assert_eq!(
                found[0].col,
                prefix.rsplit('\n').next().unwrap().chars().count() as u32 + 1
            );
            assert_eq!(
                found[0].message,
                "Duplicate element inside regexp character class"
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn extended_whitespace_including_vertical_tab_is_not_a_quantifier_target() {
        for whitespace in [" ", "\t", "\n", "\r", "\u{b}", "\u{c}"] {
            for source in [
                format!("/(?x){whitespace}*[aa]#{{value}}/"),
                format!("/{whitespace}*[aa]#{{value}}/x"),
                format!("/(?x:{whitespace}*[aa])#{{value}}/"),
            ] {
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
        }
        // VT is still literal outside extended mode and inside character sets.
        for source in [
            "/\u{b}*[aa]#{value}/",
            "/(?-x:\u{b}*[aa])#{value}/x",
            "/(?x)[\u{b}\u{b}]#{value}/",
            "/(?x)a\u{b}*[aa]#{value}/",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            let offset = if source.contains("[aa]") {
                source.find("[aa]").unwrap() + 2
            } else {
                source.find("[\u{b}\u{b}]").unwrap() + 2
            };
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, offset as u32 + 1, 1)
            );
            assert_eq!(
                found[0].message,
                "Duplicate element inside regexp character class"
            );
            assert!(found[0].correction.is_none());
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
    fn original_source_tree_nil_cases_use_the_shared_pinned_validation() {
        // Found while auditing the full original implementation: capture isolation
        // controls preserved these older set false positives, not parser gates.
        // Native3.4 -c accepts all of them; pinned regexp_parser has no tree.
        for source in [
            r"/(?<1>a)(b)[aa]/",
            r"/(?<n>a)(b)\1[aa]/",
            r"/(?<n>a)(b)(?u-a)[aa]/",
            r"/(?<n>a)(b)(?d-u:[aa])/",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [r"/(?<n>a)(b)\2[aa]/", r"/(?<n>a)(b)(?a-imx:[aa])/"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn relative_reference_arithmetic_rejects_out_of_range_without_panicking() {
        // Exact witnesses, plus both signs, delimiters,
        // dynamic fragments and values beyond the machine integer width.
        for pattern in [
            r"(?<n>a)(b)\g<+9223372036854775807>",
            r"(?:(?<n>a)(b)\g<+9223372036854775807>+)+",
            r"(a)[aa]\g<+9223372036854775807>",
            r"(a)[aa]\g'+9223372036854775807'",
            r"(a)[aa]\g<-9223372036854775808>",
            r"(a)[aa]\k<-9223372036854775808>",
            r"(a)[aa]\g<+18446744073709551616>",
            r"(a)[aa]\g<-18446744073709551616>",
            r"[aa]\g<+9223372036854775807>",
        ] {
            let mut view = super::View::empty((1, 2));
            for (i, b) in pattern.bytes().enumerate() {
                view.push(b, i, i + 1, false);
            }
            let mut scanner = super::Scanner::new(&view, false);
            assert!(scanner.scan().is_empty(), "{pattern}");
            assert!(!scanner.valid, "{pattern}");
            for source in [format!("/{pattern}/"), format!("/{pattern}#{{value}}/")] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            r"/(a)[aa]\g<-1>/",
            r"/(a)[aa]\k<-1>/",
            r"/[aa]\g<+1>(a)/",
            r"/[aa]\g<0>/",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].col, if source.starts_with("/(a)") { 7 } else { 4 });
            assert!(found[0].correction.is_none());
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
    fn property_tokens_fall_back_before_alias_validation() {
        for (source, cols) in [
            (r"/[aa]\p{foo!}#{value}/", vec![4]),
            (r"/[\p{Alpha^}aa]#{value}/", vec![13, 14]),
            (r"/[aa]\P{}#{value}/", vec![4]),
            (r"/[aa]\p{^^Alpha}#{value}/", vec![4]),
            (r"/[aa]\p{^Alpha}#{value}/", vec![4]),
            (r"/[aa]\p{Al_pha}#{value}/", vec![4]),
            (r"/[aa]\p{NotAProperty}#{value}/", vec![]),
            (r"/[aa]\p{gc=Lu}#{value}/", vec![]),
        ] {
            assert_eq!(
                offenses(source).iter().map(|o| o.col).collect::<Vec<_>>(),
                cols,
                "{source}"
            );
        }
    }
    #[test]
    fn property_normalization_includes_literal_vertical_tabs() {
        // Literal byte 0B, not \\v.
        for kind in ["p", "P"] {
            for (source, cols) in [
                (format!("/[aa]\\{kind}{{Al\u{b}pha}}#{{value}}/"), vec![4]),
                (format!("/[\\{kind}{{Al\u{b}pha}}aa]#{{value}}/"), vec![14]),
            ] {
                let found = offenses(&source);
                assert_eq!(
                    found
                        .iter()
                        .map(|o| (o.line, o.col, o.end_line))
                        .collect::<Vec<_>>(),
                    cols.into_iter().map(|col| (1, col, 1)).collect::<Vec<_>>(),
                    "{source:?}"
                );
                assert!(
                    found
                        .iter()
                        .all(|o| o.message == "Duplicate element inside regexp character class")
                );
                assert!(found.iter().all(|o| o.correction.is_none()));
            }
        }
        let found = lint_str("/[aa]\\p{Al\u{b}pha}[A-z]#{value}/");
        for (name, col) in [
            ("Lint/DuplicateRegexpCharacterClassElement", 4),
            ("Lint/MixedCaseRange", 17),
        ] {
            assert!(
                found
                    .iter()
                    .any(|o| o.cop == name && (o.line, o.col) == (1, col)),
                "{found:?}"
            );
        }
        assert!(offenses("/[aa]\\p{No\u{b}SuchProperty}#{value}/").is_empty());
    }
    #[test]
    fn control_meta_tokens_consume_raw_ascii_not_recursive_escapes() {
        // No interpolation executes.
        for (source, cols) in [
            (r"/[\c\x61\c\x61aa]#{value}/", vec![9, 13, 14, 16]),
            (r"/[\M-\x61\M-\x61aa]#{value}/", vec![10, 15, 16, 18]),
            (r"/[aa]\c\p{NoSuchProperty}#{value}/", vec![4]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line))
                    .collect::<Vec<_>>(),
                cols.into_iter().map(|col| (1, col, 1)).collect::<Vec<_>>(),
                "{source}"
            );
            assert!(found.iter().all(|o| o.message
                == "Duplicate element inside regexp character class"
                && o.correction.is_none()));
        }
        // Bare C/M and unfinished control sequences still fail the source tree.
        for source in [r"/[aa]\C#{value}/", r"/[aa]\M#{value}/"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn controlled_physical_newlines_use_pinned_emitted_fragment_ranges() {
        assert_eq!(
            offenses("/[aa]\\c\r\n[bb]#{value}/")
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            vec![(1, 4)]
        );
        assert_eq!(
            offenses("/[aa]\\c\n[bb]#{value}[cc]\n[dd]/")
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            vec![(1, 4), (2, 16)]
        );
        for prefix in [
            r"\c", r"\C-", r"\M-", r"\c\M-", r"\M-\c", r"\M-\C-", r"\C-\M-",
        ] {
            let source = format!("/[{prefix}\n{prefix}\naa]#{{value}}/");
            assert!(offenses(&source).is_empty(), "{source:?}");
            let source = format!("/(?<n>a)(b)[aa]{prefix}\n[A-z](?:x+)+#{{value}}/");
            let found = lint_str(&source);
            let selected = found
                .iter()
                .filter(|o| {
                    [
                        "Lint/DuplicateRegexpCharacterClassElement",
                        "Lint/MixedCaseRange",
                        "Lint/RedundantRegexpQuantifiers",
                    ]
                    .contains(&o.cop)
                })
                .collect::<Vec<_>>();
            assert_eq!(selected.len(), 1, "{source:?}\n{selected:?}");
            assert_eq!(
                (selected[0].cop, selected[0].line, selected[0].col),
                ("Lint/DuplicateRegexpCharacterClassElement", 1, 14)
            );
        }
    }
    #[test]
    fn named_conditions_require_the_complete_pinned_token() {
        for source in [
            r"/(?'a>b'x)[aa](?(<a>b>)a|b)#{value}/",
            r"/(?<a'b>x)[aa](?('a'b')a|b)#{value}/",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses(r"/(?'a>b'x)[aa](?('a>b')a|b)#{value}/").len(), 0);
        assert_eq!(offenses(r"/(?<a>x)[aa](?(<a>)a|b)#{value}/").len(), 1);
        for source in [
            r"/(?<ab>x)(?(<a)b>)(?:x+)+|y)z/",
            r"/(?<ab>x)(?('a>b')(?:x+)+|y)z/",
        ] {
            let found = lint_str(source);
            assert_eq!(
                found
                    .iter()
                    .filter(|o| o.cop == "Lint/RedundantRegexpQuantifiers")
                    .count(),
                1,
                "{source}"
            );
        }
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
