//! `Layout/HashAlignment` — fixed `key` style for both delimiters, every
//! last-argument hash inspected.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HashAlignment;
impl Cop for HashAlignment {
    fn name(&self) -> &'static str {
        "Layout/HashAlignment"
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
        let elements: Vec<_> = if let Some(hash) = node.as_hash_node() {
            hash.elements().iter().collect()
        } else if let Some(hash) = node.as_keyword_hash_node() {
            hash.elements().iter().collect()
        } else {
            return;
        };
        if file.line_of(node.location().start_offset()) == file.last_line_of(node.location()) {
            return;
        }
        let elements: Vec<_> = elements.iter().map(|e| Element::new(file, e)).collect();
        let Some(first_index) = elements.iter().position(|e| e.pair.is_some()) else {
            return;
        };
        let first = &elements[first_index];
        // Upstream checks the first pair's own spacing, then every element
        // (the first pair again, deduplicated by location) that begins its line.
        let mut found: Vec<(usize, Deltas)> = Vec::new();
        let mut record = |index: usize, deltas: Deltas| {
            if deltas.is_zero() || deltas.key < -(elements[index].column as i64) {
                return;
            }
            // Only the first pair can be recorded twice; it is always first.
            match found.first_mut() {
                Some(slot) if slot.0 == index => slot.1 = deltas,
                _ => found.push((index, deltas)),
            }
        };
        record(first_index, first.spacing(file));
        for (index, element) in elements.iter().enumerate() {
            if !super::alignment::starts_line(file, element.range.start) {
                continue;
            }
            let mut deltas = element.spacing(file);
            deltas.key = if first.same_line(element) {
                0
            } else {
                first.column as i64 - element.column as i64
            };
            record(index, deltas);
        }
        for (index, deltas) in found {
            let element = &elements[index];
            let (line, col) = file.line_col(element.range.start);
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| {
                        correction(
                            file,
                            element,
                            &deltas,
                            cx.ancestors().next_back().unwrap_or(node),
                        )
                    })
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: if element.pair.is_some() {
                    "Align the keys of a hash literal if they span more than one line."
                } else {
                    "Align keyword splats with the rest of the hash if it spans more than one line."
                }
                .into(),
                line,
                col,
                end_line: element.last_line,
            });
        }
    }
}
/// The fixed `key` deltas: key column, one space before a rocket and one
/// space before a value on the key's line.
#[derive(Default)]
struct Deltas {
    key: i64,
    separator: i64,
    value: i64,
}
impl Deltas {
    fn is_zero(&self) -> bool {
        self.key == 0 && self.separator == 0 && self.value == 0
    }
}
/// A pair's parser-translation anchors: the key (a label excludes its colon),
/// the operator (`=>` or the label colon) and the value, if not omitted.
struct Pair {
    key: std::ops::Range<usize>,
    operator: std::ops::Range<usize>,
    rocket: bool,
    value: Option<usize>,
}
/// A `pair` or `kwsplat` (including an anonymous `**`) hash element.
struct Element {
    range: std::ops::Range<usize>,
    first_line: u32,
    last_line: u32,
    /// Character column of the element (its key) start, as upstream `loc.column`.
    column: usize,
    pair: Option<Pair>,
}
impl Element {
    fn new(file: &SourceFile, node: &Node<'_>) -> Self {
        let loc = node.location();
        let range = loc.start_offset()..loc.end_offset();
        let pair = node.as_assoc_node().map(|assoc| {
            let key = assoc.key().location();
            let key = key.start_offset()..key.end_offset();
            let (operator, rocket) = match assoc.operator_loc() {
                Some(op) => (op.start_offset()..op.end_offset(), true),
                None => (key.end - 1..key.end, false),
            };
            let value = assoc.value();
            Pair {
                key: if rocket { key } else { key.start..key.end - 1 },
                operator,
                rocket,
                value: value
                    .as_implicit_node()
                    .is_none()
                    .then(|| value.location().start_offset()),
            }
        });
        Self {
            first_line: file.line_of(range.start),
            last_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
            column: super::alignment::effective_column(file, range.start),
            range,
            pair,
        }
    }
    /// Upstream `HashElementNode#same_line?`.
    fn same_line(&self, other: &Element) -> bool {
        self.last_line == other.first_line || self.first_line == other.last_line
    }
    /// `KeyAlignment#separator_delta` and `#value_delta` for a pair.
    fn spacing(&self, file: &SourceFile) -> Deltas {
        let Some(pair) = &self.pair else {
            return Deltas::default();
        };
        let col = |offset| super::alignment::effective_column(file, offset) as i64;
        let separator = if pair.rocket {
            col(pair.key.end) + 1 - col(pair.operator.start)
        } else {
            0
        };
        let value = match pair.value {
            Some(value) if file.line_of(value) == file.line_of(pair.key.start) => {
                col(pair.operator.end) + 1 - col(value)
            }
            _ => 0,
        };
        Deltas {
            key: 0,
            separator,
            value,
        }
    }
}
/// Upstream `correct_node`: independent insertions before, or removals of
/// the characters just before, the key, the separator and the value (or the
/// whole element). Only horizontal whitespace on one line is ever removed;
/// anything else, literal interiors and the closed source observers hold.
fn correction(
    file: &SourceFile,
    element: &Element,
    deltas: &Deltas,
    root: &Node<'_>,
) -> Option<Correction> {
    let mut edits = Vec::new();
    match &element.pair {
        Some(pair) if pair.value.is_some() => {
            edits.push((pair.key.start, deltas.key));
            edits.push((pair.operator.start, deltas.separator));
            edits.push((pair.value?, deltas.value));
        }
        _ => edits.push((element.range.start, deltas.key)),
    }
    edits.retain(|(_, delta)| *delta != 0);
    edits.sort_unstable();
    let mut ranges = Vec::new();
    for &(offset, delta) in &edits {
        let range = if delta > 0 {
            offset..offset
        } else {
            let start = offset.checked_sub(delta.unsigned_abs() as usize)?;
            if !file.bytes[start..offset]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'))
            {
                return None;
            }
            start..offset
        };
        if ranges
            .last()
            .is_some_and(|last: &std::ops::Range<usize>| last.end > range.start)
            || file.literal_interior_at(root, offset)
        {
            return None;
        }
        ranges.push(range);
    }
    let (start, end) = (ranges.first()?.start, ranges.last()?.end);
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
    let mut replacement = Vec::new();
    let mut cursor = start;
    for (range, (_, delta)) in ranges.iter().zip(&edits) {
        replacement.extend_from_slice(&file.bytes[cursor..range.start]);
        if *delta > 0 {
            replacement.extend(std::iter::repeat_n(b' ', *delta as usize));
        }
        cursor = range.end;
    }
    replacement.extend_from_slice(&file.bytes[cursor..end]);
    Some(Correction {
        range: start..end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop hash_alignment_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012–26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    const KEYS: &str = "Align the keys of a hash literal if they span more than one line.";
    const SPLAT: &str =
        "Align keyword splats with the rest of the hash if it spans more than one line.";
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == HashAlignment.name())
            .collect()
    }
    fn check(before: &str, after: &str, expected: &[(u32, u32, u32, &str)]) {
        let found: Vec<_> = offenses(before)
            .into_iter()
            .map(|o| (o.line, o.col, o.end_line, o.message))
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|(l, c, e, m)| (*l, *c, *e, m.to_string()))
            .collect();
        assert_eq!(found, expected, "{before}");
        assert_correction(&HashAlignment, before, after);
        assert!(offenses(after).is_empty(), "{after}");
    }
    #[test]
    fn always_inspect_last_argument_hashes_of_calls_super_yield_and_safe_calls() {
        for (open, close) in [
            ("func(", ")"),
            ("func({", "})"),
            ("super(", ")"),
            ("super({", "})"),
            ("yield(", ")"),
            ("yield({", "})"),
            ("foo&.bar(", ")"),
            ("foo&.bar({", "})"),
        ] {
            // Top-level `yield` is invalid Ruby 3.4; the method body is not.
            let pad = " ".repeat(open.len());
            check(
                &format!("def m\n{open}a: 0,\n  b: 1{close}\nend\n"),
                &format!("def m\n{open}a: 0,\n{pad}b: 1{close}\nend\n"),
                &[(3, 3, 3, KEYS)],
            );
        }
        for (open, close) in [("func(", ")"), ("func({", "})")] {
            let pad = " ".repeat(open.len());
            check(
                &format!("{open}a:,\n  b:{close}\n"),
                &format!("{open}a:,\n{pad}b:{close}\n"),
                &[(2, 3, 2, KEYS)],
            );
        }
    }
    #[test]
    fn default_configuration_keys_separators_and_values() {
        check(
            "hash1 = {\n  a: 0,\n   bb: 1\n}\nhash2 = {\n  'ccc' => 2,\n 'dddd'  =>  2\n}\n",
            "hash1 = {\n  a: 0,\n  bb: 1\n}\nhash2 = {\n  'ccc' => 2,\n  'dddd' => 2\n}\n",
            &[(3, 4, 3, KEYS), (7, 2, 7, KEYS)],
        );
        check(
            "hash = { a: 1, b: 2,\n        c: 3 }\n",
            "hash = { a: 1, b: 2,\n         c: 3 }\n",
            &[(2, 9, 2, KEYS)],
        );
        check(
            "hash1 = {\n  a:   0,\n  bb:1,\n}\nhash2 = {\n  'ccc'=> 2,\n  'dddd' =>  3\n}\n",
            "hash1 = {\n  a: 0,\n  bb: 1,\n}\nhash2 = {\n  'ccc' => 2,\n  'dddd' => 3\n}\n",
            &[
                (2, 3, 2, KEYS),
                (3, 3, 3, KEYS),
                (6, 3, 6, KEYS),
                (7, 3, 7, KEYS),
            ],
        );
        check(
            "hash = {\n    'a' => 0,\n  'bbb' => 1\n}\n",
            "hash = {\n    'a' => 0,\n    'bbb' => 1\n}\n",
            &[(3, 3, 3, KEYS)],
        );
        check(
            "hash = {\n  'a'   => 0,\n  'bbb' => 1\n}\n",
            "hash = {\n  'a' => 0,\n  'bbb' => 1\n}\n",
            &[(2, 3, 2, KEYS)],
        );
        check(
            "hash = {\n  'a' =>  (\n    ),\n  'bbb' => 1\n}\n",
            "hash = {\n  'a' => (\n    ),\n  'bbb' => 1\n}\n",
            &[(2, 3, 3, KEYS)],
        );
        check(
            "func(a: 0,\n  b: 1)\n",
            "func(a: 0,\n     b: 1)\n",
            &[(2, 3, 2, KEYS)],
        );
        check(
            "func(a: 0,\n   bbb: 1)\n",
            "func(a: 0,\n     bbb: 1)\n",
            &[(2, 4, 2, KEYS)],
        );
    }
    #[test]
    fn mixed_styles_and_keyword_splats() {
        check(
            "hash1 = { a: 0,\n     bb: 1,\n           ccc: 2 }\nhash2 = { :a   => 0,\n  :bb  => 1,\n            :ccc  =>2 }\nhash3 = { 'a'   =>   0,\n               'bb'  => 1,\n    'ccc'  =>2 }\n",
            "hash1 = { a: 0,\n          bb: 1,\n          ccc: 2 }\nhash2 = { :a => 0,\n          :bb => 1,\n          :ccc => 2 }\nhash3 = { 'a' => 0,\n          'bb' => 1,\n          'ccc' => 2 }\n",
            &[
                (2, 6, 2, KEYS),
                (3, 12, 3, KEYS),
                (4, 11, 4, KEYS),
                (5, 3, 5, KEYS),
                (6, 13, 6, KEYS),
                (7, 11, 7, KEYS),
                (8, 16, 8, KEYS),
                (9, 5, 9, KEYS),
            ],
        );
        check(
            "Hash(foo: 'bar',\n       **extra_params\n)\n",
            "Hash(foo: 'bar',\n     **extra_params\n)\n",
            &[(2, 8, 2, SPLAT)],
        );
        check(
            "{foo: 'bar',\n       **extra_params\n}\n",
            "{foo: 'bar',\n **extra_params\n}\n",
            &[(2, 8, 2, SPLAT)],
        );
        check(
            "def f(**)\n  g(a: 1,\n      **)\nend\n",
            "def f(**)\n  g(a: 1,\n    **)\nend\n",
            &[(3, 7, 3, SPLAT)],
        );
    }
    #[test]
    fn accepted_pinned_shapes() {
        for source in [
            "hash1 = {\n  aa: 0,\n  b: 1,\n}\nhash2 = {\n  :a => 0,\n  :bb => 1\n}\nhash3 = {\n  'a' => 0,\n  'bb' => 1\n}\n",
            "hash = {\n  'a' =>\n    0,\n  'bbb' => 1\n}\n",
            "func(a: 0,\n     b: 1)\n",
            "h = {}\n",
            "func(a: 0, bb: 1)\n",
            "func(a: 1, bb: 2,\n     ccc: 3, dddd: 4)\n",
            "render :json => {:a => messages,\n                 :b => :json}, :status => 404\ndef example\n  a(\n    b: :c,\n    d: e(\n      f: g\n    ), h: :i)\nend\n",
            "func(a:, bb:)\n",
            "func(a:, bb:,\n     ccc:, dddd:)\n",
            "render :json => {a:,\n                 b:}, :status => 404\ndef example\n  a(\n    b:,\n    c: d(\n      e:\n    ), f:)\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn yield_and_super_without_arguments_and_lookalikes_are_ignored() {
        for source in [
            "def m\n  yield\nend\n",
            "def m\n  super\nend\n",
            "case x\nin {a: 1,\n  b: 2}\nend\n",
            "{**a,\n  **b}\n",
            "h = {\n  'Ｒｕｂｙ' => 1,\n  'x' => 2\n}\n",
            "h = {\n  a:\t1,\n  b: 2\n}\n",
            "f(a, b: 1,\n     c: 2)\n",
            "x = {a: 1, b: {c: 1,\n               d: 2}}\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn index_arguments_kwsplat_first_elements_and_nested_hashes() {
        check(
            "x[a: 1,\n   b: 2]\n",
            "x[a: 1,\n  b: 2]\n",
            &[(2, 4, 2, KEYS)],
        );
        check(
            "{**a,\n  b: 1,\n c: 2}\n",
            "{**a,\n  b: 1,\n  c: 2}\n",
            &[(3, 2, 3, KEYS)],
        );
        check(
            "f(\n  **a,\n    b: 1)\n",
            "f(\n    **a,\n    b: 1)\n",
            &[(2, 3, 2, SPLAT)],
        );
        check(
            "x = {a: {c: 1,\n   d: 2},\n  b: 3}\n",
            "x = {a: {c: 1,\n         d: 2},\n     b: 3}\n",
            &[(2, 4, 2, KEYS), (3, 3, 3, KEYS)],
        );
    }
    #[test]
    fn corrections_hold_for_closed_observers_and_literal_interiors() {
        for source in [
            "h = {\n  a: 1,\n   b: 2\n}\np __FILE__\n",
            "h = {\n  a: 1,\n   b: 2\n}\np DATA.read\n__END__\nx\n",
            "x = <<~T\n  #{ {a: 1,\n       b: 2} }\nT\n",
        ] {
            let found = correction_offenses(&HashAlignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        // Readers outside the closed observer list do not hold the edit.
        check(
            "h = {\n  a: 1,\n   b: 2\n}\np File.read('h.rb')\n",
            "h = {\n  a: 1,\n  b: 2\n}\np File.read('h.rb')\n",
            &[(3, 4, 3, KEYS)],
        );
    }
    #[test]
    fn continued_separators_and_multiline_keys_use_upstream_columns() {
        check(
            "h = {\n  'a' \\\n        => 1,\n  'b' => 2\n}\n",
            "h = {\n  'a' \\\n      => 1,\n  'b' => 2\n}\n",
            &[(2, 3, 3, KEYS)],
        );
        check(
            "h = {\n  \"a\n\"  => 1,\n  'b' => 2\n}\n",
            "h = {\n  \"a\n\" => 1,\n  'b' => 2\n}\n",
            &[(2, 3, 3, KEYS)],
        );
    }
    #[test]
    fn wide_hashes_report_and_correct_each_pair_once() {
        let source = format!("h = {{\n  a0: 0,\n{}}}\n", "   b: 1,\n".repeat(5000));
        let found = correction_offenses(&HashAlignment, &source);
        assert_eq!(found.len(), 5000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        let source = "h = {\n  a:  0,\n  b: 1\n}\n";
        let found = correction_offenses(&HashAlignment, source);
        assert_eq!(found.len(), 1);
    }
}
