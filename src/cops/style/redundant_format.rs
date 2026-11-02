//! `Style/RedundantFormat` — detection-only; formatting can change mutability.
use super::format_string_token::TOKENS;
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{collections::HashMap, sync::LazyLock};

static FLOAT_STRING: LazyLock<regex::Regex> = LazyLock::new(|| {
    let d = "[0-9](?:_?[0-9])*";
    let h = "[0-9a-fA-F](?:_?[0-9a-fA-F])*";
    regex::Regex::new(&format!(r"\A[+-]?(?:(?:{d}(?:\.{d})?|\.{d})(?:[eE][+-]?{d})?|0[xX]{h}(?:[pP][+-]?{d}|\.{h}[pP][+-]?{d})?)\z")).unwrap()
});

pub struct RedundantFormat;
impl Cop for RedundantFormat {
    fn name(&self) -> &'static str {
        "Style/RedundantFormat"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let method = call.name().as_slice();
        if !matches!(method, b"format" | b"sprintf") || call.is_safe_navigation() {
            return;
        }
        let args = ast::call_arguments(&call);
        let Some(first) = args.first() else { return };
        let single = args.len() == 1
            && call
                .receiver()
                .is_none_or(|r| ast::is_global_const(&r, &["Kernel"]))
            && matches!(
                first,
                Node::StringNode { .. }
                    | Node::InterpolatedStringNode { .. }
                    | Node::SourceFileNode { .. }
                    | Node::SourceEncodingNode { .. }
                    | Node::ConstantReadNode { .. }
                    | Node::ConstantPathNode { .. }
            );
        let message = if single {
            if static_fragments(first).is_some_and(|s| TOKENS.is_match(&s)) {
                return;
            }
            format!(
                "Use `{}` directly instead of `{}`.",
                escape_controls(first.location().as_slice()),
                String::from_utf8_lossy(method)
            )
        } else {
            let Some(string) = first.as_string_node() else {
                return;
            };
            if string
                .opening_loc()
                .is_some_and(|l| l.as_slice().starts_with(b"<<"))
                || args.len() < 2
                || !literal_fields(string.unescaped(), &args[1..])
            {
                return;
            }
            // Detection is not a Ruby formatter or an unsafe correction. Avoid
            // manufacturing replacement source (and potentially enormous width
            // padding) just for the message. This path matches other receivers
            // too, as upstream's detect_unnecessary_fields does.
            format!(
                "Use literal values directly instead of `{}`.",
                String::from_utf8_lossy(method)
            )
        };
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction: None,
        });
    }
}
fn escape_controls(bytes: &[u8]) -> String {
    let mut out = String::new();
    for c in String::from_utf8_lossy(bytes).chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0b' => out.push_str("\\v"),
            '\x0c' => out.push_str("\\f"),
            '\x1b' => out.push_str("\\e"),
            c if c.is_control() => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}
fn static_fragments(node: &Node<'_>) -> Option<Vec<u8>> {
    if let Some(s) = node.as_string_node() {
        return Some(s.unescaped().to_vec());
    }
    node.as_interpolated_string_node().map(|s| {
        let mut bytes = Vec::new();
        for part in s.parts().iter() {
            if let Some(part) = part.as_string_node() {
                bytes.extend_from_slice(part.unescaped());
            }
        }
        bytes
    })
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    Numbered,
    Named,
}
#[derive(Clone, Copy)]
enum Dimension {
    Fixed(usize),
    Star(Option<usize>),
}
struct Field<'a> {
    kind: u8,
    name: Option<&'a [u8]>,
    number: Option<usize>,
    width: Option<Dimension>,
    precision: Option<Dimension>,
}
fn number(bytes: &[u8], at: &mut usize) -> Option<usize> {
    let start = *at;
    let mut n = Some(0usize);
    while let Some(b) = bytes.get(*at).filter(|b| b.is_ascii_digit()) {
        n = n.and_then(|n| n.checked_mul(10)?.checked_add(usize::from(*b - b'0')));
        *at += 1;
    }
    if *at == start { None } else { n }
}
fn dimension(bytes: &[u8], at: &mut usize) -> Option<Dimension> {
    if bytes.get(*at) == Some(&b'*') {
        *at += 1;
        let start = *at;
        let n = number(bytes, at);
        if *at != start {
            if bytes.get(*at) != Some(&b'$') {
                return None;
            }
            *at += 1;
            Some(Dimension::Star(Some(n.unwrap_or(usize::MAX))))
        } else {
            Some(Dimension::Star(None))
        }
    } else {
        let start = *at;
        let value = number(bytes, at).unwrap_or(usize::MAX);
        (*at != start).then_some(Dimension::Fixed(value))
    }
}
fn field<'a>(caps: &regex::bytes::Captures<'a>) -> Option<Field<'a>> {
    let token = caps.get(0)?.as_bytes();
    if token == b"%%" {
        return None;
    }
    let named = ["template", "name_before", "name_middle", "name_after"]
        .iter()
        .find_map(|n| caps.name(n));
    let name = named.map(|n| &n.as_bytes()[1..n.len() - 1]);
    let template = caps.name("template").is_some();
    // Strip the already recognized annotation, retaining the exact other
    // syntax for validation. A template has an implicit s type.
    let mut bytes = token.to_vec();
    if let Some(n) = named {
        let begin = n.start() - caps.get(0)?.start();
        bytes.drain(begin..begin + n.len());
    }
    if template {
        bytes.push(b's');
    }
    let mut at = 1;
    let mut arg = None;
    loop {
        match bytes.get(at) {
            Some(b' ' | b'#' | b'0' | b'+' | b'-') => at += 1,
            Some(b'1'..=b'9') => {
                let start = at;
                let n = number(&bytes, &mut at);
                if bytes.get(at) == Some(&b'$') {
                    if arg.is_some() {
                        return None;
                    }
                    arg = Some(n.unwrap_or(usize::MAX));
                    at += 1;
                } else {
                    at = start;
                    break;
                }
            }
            _ => break,
        }
    }
    let width = dimension(&bytes, &mut at);
    let precision = if bytes.get(at) == Some(&b'.') {
        at += 1;
        Some(dimension(&bytes, &mut at).unwrap_or(Dimension::Fixed(0)))
    } else {
        None
    };
    if [width, precision]
        .into_iter()
        .flatten()
        .any(|d| matches!(d, Dimension::Fixed(n) if n > i32::MAX as usize))
    {
        return None;
    }
    let kind = *bytes.get(at)?;
    if at + 1 != bytes.len() || !b"sdiuf".contains(&kind) || (name.is_some() && arg.is_some()) {
        return None;
    }
    Some(Field {
        kind,
        name,
        number: arg,
        width,
        precision,
    })
}
fn acceptable(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::SymbolNode { .. }
            | Node::InterpolatedSymbolNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
    )
}
fn matches_field(field: &Field<'_>, node: &Node<'_>, facts: &mut NumericFacts) -> bool {
    match field.kind {
        b's' => {
            acceptable(node)
                && (!matches!(node, Node::InterpolatedStringNode { .. })
                    || (field.width.is_none() && field.precision.is_none()))
        }
        b'd' | b'i' | b'u' => facts.numeric(node, false),
        b'f' => facts.numeric(node, true),
        _ => false,
    }
}
fn numeric(node: &Node<'_>, float: bool) -> bool {
    if let Some(p) = node.as_parentheses_node() {
        return p
            .body()
            .and_then(|n| n.as_statements_node())
            .is_some_and(|s| {
                let body = s.body();
                body.len() == 1
                    && body.iter().next().is_some_and(|n| {
                        // Upstream unwraps exactly one begin around rational/complex
                        // literals or its special integer / rational, integer + complex
                        // forms, not ordinary parenthesized ints/strings.
                        (n.as_rational_node().is_some()
                            || n.as_imaginary_node().is_some()
                            || n.as_call_node().is_some())
                            && numeric(&n, float)
                    })
            });
    }
    match node {
        Node::IntegerNode { .. } | Node::RationalNode { .. } | Node::SourceLineNode { .. } => true,
        Node::FloatNode { .. } => float || node.as_float_node().unwrap().value().is_finite(),
        Node::ImaginaryNode { .. } => zero_imaginary(node),
        Node::StringNode { .. } => {
            numeric_string(node.as_string_node().unwrap().unescaped(), float)
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            if call.is_safe_navigation() || call.block().is_some() {
                return false;
            }
            let args = ast::call_arguments(&call);
            let Some(r) = call.receiver() else {
                return false;
            };
            let [a] = args.as_slice() else { return false };
            (call.name().as_slice() == b"/"
                && r.as_integer_node().is_some()
                && a.as_rational_node().is_some())
                || (call.name().as_slice() == b"+"
                    && r.as_integer_node().is_some()
                    && a.as_imaginary_node().is_some()
                    && zero_imaginary(a))
        }
        _ => false,
    }
}
fn zero_imaginary(node: &Node<'_>) -> bool {
    node.as_imaginary_node().is_some_and(|n| {
        let n = n.numeric();
        if let Some(n) = n.as_integer_node() {
            n.value().to_u32_digits().1.iter().all(|d| *d == 0)
        } else {
            // Complex#to_i / #to_f require an exact imaginary zero. Float
            // +0.0 and -0.0 are inexact even though they compare equal to 0;
            // retain the main AST's numeric type rather than comparing values.
            n.as_rational_node()
                .is_some_and(|n| n.numerator().to_u32_digits().1.iter().all(|d| *d == 0))
        }
    })
}
fn numeric_string(bytes: &[u8], float: bool) -> bool {
    let Ok(s) = std::str::from_utf8(bytes) else {
        return false;
    };
    let s = s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b');
    if float {
        return FLOAT_STRING.is_match(s);
    }
    if s.is_empty() || s.starts_with('_') || s.ends_with('_') || s.contains("__") {
        return false;
    }
    let clean = s.replace('_', "");
    let unsigned = clean.strip_prefix(['+', '-']).unwrap_or(&clean);
    let (digits, radix) = if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
        (&unsigned[2..], 16)
    } else if unsigned.starts_with("0b") || unsigned.starts_with("0B") {
        (&unsigned[2..], 2)
    } else if unsigned.starts_with("0o") || unsigned.starts_with("0O") {
        (&unsigned[2..], 8)
    } else if unsigned.starts_with("0d") || unsigned.starts_with("0D") {
        (&unsigned[2..], 10)
    } else if !float && unsigned.len() > 1 && unsigned.starts_with('0') {
        (unsigned, 8)
    } else {
        (unsigned, 10)
    };
    let valid_digit = |b: u8| char::from(b).is_digit(radix);
    if s.as_bytes()
        .windows(2)
        .any(|w| (w[0] == b'_' && !valid_digit(w[1])) || (w[1] == b'_' && !valid_digit(w[0])))
    {
        return false;
    }
    !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix))
}
fn literal_fields<'pr>(string: &[u8], args: &[Node<'pr>]) -> bool {
    if args.iter().any(|n| n.as_splat_node().is_some()) {
        return false;
    }
    let mut named = HashMap::new();
    let mut actual_named = HashMap::new();
    let mut facts = NumericFacts::default();
    for arg in args {
        let elements = if let Some(h) = arg.as_hash_node() {
            Some(h.elements())
        } else {
            arg.as_keyword_hash_node().map(|h| h.elements())
        };
        if let Some(elements) = elements {
            for entry in elements.iter() {
                let Some(pair) = entry.as_assoc_node() else {
                    return false;
                };
                if let Some(key) = pair.key().as_symbol_node() {
                    named
                        .entry(key.unescaped().to_vec())
                        .or_insert_with(|| pair.value());
                    actual_named.insert(key.unescaped().to_vec(), pair.value());
                }
            }
            break;
        }
    }
    let mut mode = None;
    let mut next = 0usize;
    let mut selection = RemainingArguments::new(args.len());
    let mut covered = 0;
    let mut any = false;
    for caps in TOKENS.captures_iter(string) {
        let token = caps.get(0).unwrap();
        // Ruby would raise for an unrecognized percent directive, even when
        // the permissive upstream token regexp finds later valid fields.
        if string[covered..token.start()].contains(&b'%') {
            return false;
        }
        covered = token.end();
        let Some(field) = field(&caps) else {
            return false;
        };
        any = true;
        // First perform RuboCop's literal-field selection (not Ruby's actual
        // argument consumption). Width deletes one argument before shifting.
        let selected = if let Some(name) = field.name {
            // Ruby requires exactly one hash after the format string for
            // named fields; finding a usable hash among extra args is not enough.
            if args.len() != 1 {
                return false;
            }
            named.get(name)
        } else if let Some(Dimension::Star(n)) = field.width {
            let Some(i) = n.unwrap_or(1).checked_sub(1).and_then(|n| selection.get(n)) else {
                return false;
            };
            if !facts.numeric(&args[i], true) {
                return false;
            }
            selection.remove(i);
            selection.pop().map(|i| &args[i])
        } else if let Some(n) = field.number {
            n.checked_sub(1)
                .and_then(|i| selection.get(i))
                .map(|i| &args[i])
        } else {
            selection.pop().map(|i| &args[i])
        };
        if !selected.is_some_and(|n| matches_field(&field, n, &mut facts)) {
            return false;
        }
        // Then validate the reference evaluator's actual argument consumption;
        // e.g. %*.*f consumes both width and precision before the value. Its
        // argument_value represents unknown expressions by their source, so
        // s accepts them here (the literal-field selection above is separate).
        let this_mode = if field.name.is_some() {
            Mode::Named
        } else if field.number.is_some() {
            Mode::Numbered
        } else {
            Mode::Plain
        };
        if mode.is_some_and(|m| m != this_mode) {
            return false;
        }
        mode = Some(this_mode);
        let mut get = |number: Option<usize>| -> Option<&Node<'pr>> {
            if let Some(n) = number {
                n.checked_sub(1).and_then(|i| args.get(i))
            } else {
                let value = args.get(next);
                next += 1;
                value
            }
        };
        for dim in [field.width, field.precision].into_iter().flatten() {
            if let Dimension::Star(n) = dim
                && (this_mode == Mode::Named
                    || (n.is_some() != (this_mode == Mode::Numbered))
                    || !get(n)
                        .is_some_and(|n| facts.numeric(n, false) && n.as_string_node().is_none()))
            {
                return false;
            }
        }
        let value = if let Some(name) = field.name {
            actual_named.get(name)
        } else {
            get(field.number)
        };
        if !value.is_some_and(|n| {
            if field.kind == b's' {
                return true;
            }
            // argument_value unwraps one begin even though all_fields_literal?
            // only permits special rational/complex begins during selection.
            let value = n
                .as_parentheses_node()
                .and_then(|p| p.body())
                .and_then(|b| b.as_statements_node())
                .and_then(|s| s.body().iter().next());
            let value = value.as_ref().unwrap_or(n);
            value.as_parentheses_node().is_none() && matches_field(&field, value, &mut facts)
        }) {
            return false;
        }
    }
    any && !string[covered..].contains(&b'%')
}

/// Per-call, demand-driven conversion facts. A repeated numbered field never
/// rescans a large decoded literal. No formatting/padding allocations are made.
#[derive(Default)]
struct NumericFacts(HashMap<(usize, bool), bool>);
impl NumericFacts {
    fn numeric(&mut self, node: &Node<'_>, float: bool) -> bool {
        *self
            .0
            .entry((node.location().start_offset(), float))
            .or_insert_with(|| numeric(node, float))
    }
}
/// A Fenwick live-index tree: upstream shifts and deletes arguments as it
/// matches fields. Vec::remove(0) would make a long flat call quadratic.
struct RemainingArguments {
    tree: Vec<usize>,
    len: usize,
}
impl RemainingArguments {
    fn new(len: usize) -> Self {
        Self {
            tree: (0..=len).map(usize::isolate_lowest_one).collect(),
            len,
        }
    }
    fn get(&self, rank: usize) -> Option<usize> {
        if rank >= self.len {
            return None;
        }
        let mut remaining = rank + 1;
        let mut index = 0;
        let mut step = self.tree.len().next_power_of_two() / 2;
        while step != 0 {
            let next = index + step;
            if next < self.tree.len() && self.tree[next] < remaining {
                index = next;
                remaining -= self.tree[next];
            }
            step /= 2;
        }
        Some(index)
    }
    fn remove(&mut self, index: usize) {
        let mut i = index + 1;
        while i < self.tree.len() {
            self.tree[i] -= 1;
            i += i.isolate_lowest_one();
        }
        self.len -= 1;
    }
    fn pop(&mut self) -> Option<usize> {
        let i = self.get(0)?;
        self.remove(i);
        Some(i)
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/redundant_format_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantFormat")
            .collect()
    }
    fn assert_count(s: &str, count: usize) {
        let found = offenses(s);
        assert_eq!(found.len(), count, "{s}");
        for o in found {
            assert_eq!((o.line, o.col), (1, 1), "{s}");
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn single_argument_exact_diagnostics() {
        for method in ["format", "sprintf"] {
            for (arg, prefer) in [
                ("'foo'", "'foo'"),
                (r##""#{foo}""##, r##""#{foo}""##),
                ("MESSAGE", "MESSAGE"),
                ("Other::MESSAGE", "Other::MESSAGE"),
                (r#""\n""#, r#""\n""#),
                ("<<~MSG\n foo\nMSG\n", "<<~MSG"),
            ] {
                let s = if arg.starts_with("<<") {
                    format!("{method}({prefer})\n foo\nMSG\n")
                } else {
                    format!("{method}({arg})")
                };
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    format!("Use `{prefer}` directly instead of `{method}`.")
                );
            }
            for receiver in ["Kernel.", "::Kernel."] {
                assert_count(&format!("{receiver}{method}('foo')"), 1);
            }
        }
        let found = offenses("# hi\n  format('foo')");
        assert_eq!((found[0].line, found[0].col), (2, 3));
    }
    #[test]
    fn single_argument_source_encoding_exact_diagnostics() {
        for receiver in ["", "Kernel.", "::Kernel."] {
            for method in ["format", "sprintf"] {
                let source = format!("{receiver}{method}(__ENCODING__)");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!(
                    found[0].message,
                    format!("Use `__ENCODING__` directly instead of `{method}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
        assert_count("Other.format(__ENCODING__)", 0);
        assert_count("Kernel&.format(__ENCODING__)", 0);
    }
    #[test]
    fn named_formats_require_exactly_one_hash_argument() {
        for method in ["format", "sprintf"] {
            for spec in ["%<x>s", "%{x}", "%<x>d"] {
                for args in [
                    "{x: 1}, {x: 2}",
                    "{x: 1}, 2",
                    "2, {x: 1}",
                    "2, x: 1",
                    "{x: 1}, x: 2",
                ] {
                    assert_count(&format!("{method}('{spec}', {args})"), 0);
                }
                for args in ["{x: 1}", "x: 1"] {
                    assert_count(&format!("{method}('{spec}', {args})"), 1);
                }
            }
        }
    }
    #[test]
    fn decorated_calls_are_not_numeric_literals() {
        for method in ["format", "sprintf"] {
            for spec in ["%d", "%i", "%u", "%f"] {
                for arg in [
                    "1&.+(0i)",
                    "1.+(0i) { x }",
                    "1&./(2r)",
                    "1./(2r) { x }",
                    "(1&.+(0i))",
                    "(1.+(0i) { x })",
                    "(1&./(2r))",
                    "(1./(2r) { x })",
                ] {
                    assert_count(&format!("{method}('{spec}', {arg})"), 0);
                }
                for arg in ["1+0i", "1/2r"] {
                    assert_count(&format!("{method}('{spec}', {arg})"), 1);
                }
            }
        }
    }
    #[test]
    fn floating_imaginary_zero_is_not_convertible_to_integer_or_float() {
        for method in ["format", "sprintf"] {
            // String formatting does not perform numeric conversion and still
            // accepts inexact imaginary zero as an ordinary complex literal.
            for arg in ["0.0i", "-0.0i", "0e2i"] {
                assert_count(&format!("{method}('%s', {arg})"), 1);
                assert_count(&format!("{method}('%{{x}}', x: {arg})"), 1);
            }
            for arg in [
                "0.0i",
                "-0.0i",
                "0e2i",
                "-0e2i",
                "(0.0i)",
                "(-0.0i)",
                "5+0.0i",
                "5+-0.0i",
                "(5+0.0i)",
                "(5+-0.0i)",
                "5+0e2i",
            ] {
                for spec in ["%d", "%i", "%u", "%f", "%-2d", "%10.2f"] {
                    assert_count(&format!("{method}('{spec}', {arg})"), 0);
                }
                for (spec, args) in [
                    ("%f/%s", format!("{arg}, :foo")),
                    ("%<x>d", format!("{{x: {arg}}}")),
                    ("%<x>f", format!("x: {arg}")),
                ] {
                    assert_count(&format!("{method}('{spec}', {args})"), 0);
                }
            }
        }
    }
    #[test]
    fn floating_imaginary_zero_cannot_supply_star_width_or_precision() {
        // Some of these shapes raise TypeError inside the reference cop. They
        // are no-crash/no-offense regressions, not reference-parity evidence.
        for arg in ["0.0i", "-0.0i", "(0.0i)", "5+0.0i", "(5+0.0i)"] {
            for (spec, args) in [
                ("%*d", format!("{arg}, 1")),
                ("%.*f", format!("{arg}, 1")),
                ("%*.*f", format!("{arg}, 2, 1")),
                ("%*.*f", format!("5, {arg}, 1")),
                ("%1$*2$d", format!("1, {arg}")),
                ("%1$.*2$f", format!("1, {arg}")),
            ] {
                assert_count(&format!("format('{spec}', {args})"), 0);
            }
        }
    }
    #[test]
    fn exact_imaginary_zero_and_pinned_rational_forms_remain_convertible() {
        for method in ["format", "sprintf"] {
            for arg in [
                "0i", "-0i", "0x0i", "0ri", "0.0ri", "(0i)", "(0ri)", "5+0i", "(5+0i)", "5+0ri",
                "(5+0ri)", "5r", "3/8r", "(3/8r)",
            ] {
                for spec in ["%d", "%i", "%u", "%f", "%-2d"] {
                    let source = format!("  {method}('{spec}', {arg})");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (1, 3), "{source}");
                    assert_eq!(
                        found[0].message,
                        format!("Use literal values directly instead of `{method}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
                for (spec, args) in [
                    ("%<x>d", format!("{{x: {arg}}}")),
                    ("%*d", format!("{arg}, 1")),
                    ("%*.*f", format!("5, {arg}, 1")),
                ] {
                    assert_count(&format!("{method}('{spec}', {args})"), 1);
                }
            }
        }
    }
    #[test]
    fn literal_types_and_numeric_conversions() {
        for method in ["format", "sprintf"] {
            for arg in [
                "'foo'",
                "%q{foo}",
                "%{foo}",
                "%Q{foo}",
                r##""#{foo}""##,
                ":foo",
                r##":"#{foo}""##,
                "1",
                "1.1",
                "1r",
                "1i",
                "true",
                "false",
                "nil",
            ] {
                // Dynamic symbols have ordinary Ruby quoted-symbol spelling.
                let arg = arg.replace("\\\"", "\"");
                assert_count(&format!("{method}('%s', {arg})"), 1);
            }
            for spec in ["%d", "%i", "%u", "%f"] {
                for arg in [
                    "5", "5.5", "5r", "3/8r", "(3/8r)", "5+0i", "(5+0i)", "'5'", "'-5'",
                ] {
                    assert_count(&format!("{method}('{spec}', {arg})"), 1);
                }
                for arg in ["'abcd'", "true", "false", "nil", "[]", "{}"] {
                    assert_count(&format!("{method}('{spec}', {arg})"), 0);
                }
            }
            for spec in ["%d", "%i", "%u"] {
                assert_count(&format!("{method}('{spec}', '5.5')"), 0);
            }
        }
    }
    #[test]
    fn width_precision_numbering_named_and_templates() {
        for (spec, args) in [
            ("%10s", "'foo'"),
            ("%-10s", "'foo'"),
            ("%10d", "5"),
            ("%-10d", "5"),
            ("% d", "5"),
            ("%+d", "5"),
            ("%.3d", "10"),
            ("%.d", "0"),
            ("%05d", "5"),
            ("%.2f", "5"),
            ("%10.2f", "5"),
            ("%-10.2f", "5"),
            ("%2$s %1$s", "'world', 'hello'"),
            ("%*d", "5, 14"),
            ("$%0*.*f", "5, 2, 0.5"),
            ("%-*d", "5, 14"),
            ("%1$*2$s", "14, 5"),
            ("%1$-*2$s", "14, 5"),
            ("%<foo>s %<bar>s", "foo: 'foo', bar: 'bar'"),
            ("%<foo>s %<bar>s", "foo: \"#{foo}\", bar: 'bar'"),
            ("%{foo}s %{bar}s", "foo: 'foo', bar: 'bar'"),
            ("%{foo}", "foo: nil"),
            ("%s %s", "'foo', 'bar'"),
        ] {
            assert_count(&format!("format('{spec}', {args})"), 1);
        }
        for (spec, args) in [
            ("%10s", "\"#{foo}\""),
            ("%.1s", "\"#{foo}\""),
            ("%% %s", "foo"),
            ("%2$s %1$i", "'abcd', '5'"),
            ("%999999999999999999999999999999999999$d", "1"),
            ("%*9$d", "1"),
            ("%*99999999999999999999999999999999$d", "1"),
            ("%*d", "'a', 'foo'"),
            ("%*s", "foo, 'bar'"),
            ("%-*s", "foo, 'bar'"),
            ("%*d", "5, foo"),
            ("%*.*d", "5, 2, foo"),
            ("%<foo>s", "foo: foo"),
            ("%<foo>?%<bar>s:%<baz>s", "'foobar'"),
            ("%{foo}", "foo"),
            ("%{foo}", "foo: foo"),
            ("%s %s", "'foo', bar"),
            ("%{y}-%{m}-%{d}", "2015, 1, 1"),
        ] {
            assert_count(&format!("format('{spec}', {args})"), 0);
        }
    }
    #[test]
    fn non_offenses_receiver_arguments_splats_and_heredocs() {
        for s in [
            "format",
            "format()",
            "format('%s', foo)",
            "format('%s', *args)",
            "format('%s', **args)",
            "format('%<username>s', **args)",
            "foo.format('bar')",
            "format('%%')",
            "format('%s')",
            "format(FORMAT, 'foo', 'bar')",
            "format(*FORMAT)",
            "format(\"%0#{width}d\", 3)",
            "format(<<~MSG, 1)\n %d\nMSG\n",
            "format(<<~MSG, greeting: 'Hi')\n %<greeting>s\nMSG\n",
            "format('foo', &block)",
            "Kernel&.format('foo')",
            "format(:foo)",
            "format(1)",
            "format(x)",
            "Other::Kernel.format('foo')",
        ] {
            assert_count(s, 0);
        }
    }
    #[test]
    fn lowest_set_bit_zero_powers_and_usize_boundaries() {
        assert_eq!(0usize.isolate_lowest_one(), 0);
        for bit in 0..usize::BITS {
            let power = 1usize << bit;
            assert_eq!(power.isolate_lowest_one(), power);
            assert_eq!((usize::MAX << bit).isolate_lowest_one(), power);
        }
        assert_eq!(usize::MAX.isolate_lowest_one(), 1);
        assert_eq!((usize::MAX - 1).isolate_lowest_one(), 2);
    }

    #[test]
    fn remaining_arguments_empty_and_power_of_two_boundaries() {
        use super::RemainingArguments;

        for len in [0, 1, 2, 3, 7, 8, 9, 31, 32, 33, 63, 64, 65] {
            let mut remaining = RemainingArguments::new(len);
            assert_eq!(remaining.tree[0], 0);
            for i in 1..=len {
                assert_eq!(remaining.tree[i], 1usize << i.trailing_zeros());
            }
            let mut expected: Vec<_> = (0..len).collect();
            while !expected.is_empty() {
                for (rank, &index) in expected.iter().enumerate() {
                    assert_eq!(remaining.get(rank), Some(index));
                }
                assert_eq!(remaining.get(expected.len()), None);
                assert_eq!(remaining.get(usize::MAX), None);
                let rank = expected.len() / 2;
                let index = remaining.get(rank).unwrap();
                assert_eq!(index, expected.remove(rank));
                remaining.remove(index);
                assert_eq!(remaining.len, expected.len());
            }
            assert_eq!(remaining.get(0), None);
            assert_eq!(remaining.pop(), None);
        }
    }

    #[test]
    fn bounded_argument_selection_and_lazy_literal_facts() {
        use super::*;
        let mut remaining = RemainingArguments::new(8192);
        let mut expected: Vec<_> = (0..8192).collect();
        for i in (0..8192).rev() {
            let rank = i / 2;
            let selected = remaining.get(rank).unwrap();
            assert_eq!(selected, expected.remove(rank));
            remaining.remove(selected);
        }
        assert!(remaining.pop().is_none());
        let parsed = ruby_prism::parse(b"'1234567890'");
        let node = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap();
        let mut facts = NumericFacts::default();
        assert!(facts.0.is_empty());
        for _ in 0..10000 {
            assert!(facts.numeric(&node, false));
        }
        assert_eq!(facts.0.len(), 1);
        let source = format!(
            "format('{}', {})",
            "%d ".repeat(2000),
            vec!["1"; 2000].join(",")
        );
        assert_count(&source, 1);
        let source = format!(
            "format('{}', '{}')",
            "%1$d ".repeat(2000),
            "1".repeat(10000)
        );
        assert_count(&source, 1);
        // No attempt to allocate/render 2GB of padding just for a message.
        assert_count("format('%2147483647s', 'x')", 1);
    }
    #[test]
    fn conversion_grammar_and_duplicate_named_keys() {
        for s in [
            "format('%d', (5))",
            "format('%f', (5.5))",
            "format('%d', ((5r)))",
            "format('%d', 0e2i)",
            "format('%d', 5+0e2i)",
            "format('%2147483648s', 'x')",
            "format('%.2147483648s', 'x')",
            "format('%f', '0b1')",
            "format('%f', '0o5')",
            "format('%f', '1.')",
            "format('%f', '0x1.2')",
            "format('%f', '0x1.p2')",
            "format('%f', 'Infinity')",
            "format('%f', '1_e2')",
            "format('%f', '1e_2')",
            "format('%f', '0x_5')",
            "format('%<x>d', x: 1, x: 'bad')",
            "format('%<x>.2f', x: 1, x: 'bad')",
        ] {
            assert_count(s, 0);
        }
        for s in [
            "format('%f', '1_2e2')",
            "format('%<x>d', x: 1, x: (5))",
            "format('%<x>f', x: 1, x: (5))",
            "format('%.*s', 3, foo)",
            "format('%<x>s', x: 1, x: foo)",
            "format('%f', '0xff')",
            "format('%f', '0x1.2p2')",
            "format('%f', 0x0i)",
            "format('%f', 0.0ri)",
            "format('%f', '.1')",
            "format('%f', '\x0b1')",
            "format('%d', '1\x0b')",
            "format('%f', '1e9999')",
            "format('%<x>s', x: 'a', x: 'b')",
        ] {
            assert_count(s, 1);
        }
    }
    #[test]
    fn decoded_percent_sequences_and_fragments() {
        for s in [
            r#"format("\x25s")"#,
            r#"format("\045%")"#,
            r#"format("%" "%s")"#,
            r##"format("#{x}%s")"##,
            "format('%% %s', 'x')",
        ] {
            assert_count(s, 0);
        }
        for s in [
            r#"format("\x25s", 'x')"#,
            "format('%q')",
            "format('100%')",
            r#"format("%" "s", 'x')"#,
        ] {
            // Implicit literal concatenation is a dstr upstream, not a str.
            assert_count(s, usize::from(!s.contains("\"s\", 'x'")));
        }
    }
}
