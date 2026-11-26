//! `Lint/LiteralAssignmentInCondition` — literal equals-assignments in predicates.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};

/// Detection only: replacing assignment by comparison would change behavior.
pub struct LiteralAssignmentInCondition;
impl Cop for LiteralAssignmentInCondition {
    fn name(&self) -> &'static str {
        "Lint/LiteralAssignmentInCondition"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((operator, rhs)) = assignment(node) else {
            return;
        };
        if !all_literals(&rhs, file) || !in_condition(node, cx, file) {
            return;
        }
        let start = operator.start_offset();
        let end = ast::source_end(&rhs);
        let literal = file.match_pattern_source(&file.bytes[rhs.location().start_offset()..end]);
        let literal = String::from_utf8_lossy(&literal).replace("\r\n", "\n");
        let (line, col) = file.reference_line_col(start);
        out.push(Offense { cop: self.name(), severity: self.severity(), correction: None,
            message: format!("Don't use literal assignment `= {literal}` in conditional, should be `==` or non-literal operand."),
            line, col, end_line: file.line_of(end.saturating_sub(1).max(start)) });
    }
}

fn assignment<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Node<'pr>)> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => {
            let n = node.as_local_variable_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::InstanceVariableWriteNode { .. } => {
            let n = node.as_instance_variable_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::ClassVariableWriteNode { .. } => {
            let n = node.as_class_variable_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::GlobalVariableWriteNode { .. } => {
            let n = node.as_global_variable_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::ConstantWriteNode { .. } => {
            let n = node.as_constant_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::ConstantPathWriteNode { .. } => {
            let n = node.as_constant_path_write_node()?;
            (n.operator_loc(), n.value())
        }
        Node::MultiWriteNode { .. } => {
            let n = node.as_multi_write_node()?;
            (n.operator_loc(), n.value())
        }
        _ => return None,
    })
}

fn all_literals(node: &Node<'_>, file: &SourceFile) -> bool {
    if let Some(a) = node.as_array_node() {
        let opening = a.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
        let words = opening.starts_with(b"%w") || opening.starts_with(b"%W");
        return a.elements().iter().all(|n| {
            if words && n.as_string_node().is_some() {
                return true;
            }
            if words && let Some(s) = n.as_interpolated_string_node() {
                return matches!(
                    composed_literal(
                        &s.parts().iter().collect::<Vec<_>>(),
                        opening,
                        "str",
                        file,
                        LiteralProfile::Scope
                    ),
                    ScopeLiteral::Str(_)
                );
            }
            all_literals(&n, file)
        });
    }
    if let Some(h) = node.as_hash_node() {
        // HashNode#keys/#values deliberately ignore kwsplat nodes, even **x.
        return h.elements().iter().all(|n| {
            n.as_assoc_node()
                .is_none_or(|p| all_literals(&p.key(), file) && all_literals(&p.value(), file))
        });
    }
    if let Some(s) = node.as_string_node() {
        let opening = s.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
        if opening.starts_with(b"<<") {
            return heredoc_literal(&[s.as_node()], opening, file);
        }
        if opening.starts_with(b"%") && s.unescaped().is_empty() {
            return false;
        }
        return emitted_strings(&s, opening, file).len() == 1;
    }
    if let Some(s) = node.as_interpolated_string_node() {
        return s
            .opening_loc()
            .filter(|o| o.as_slice().starts_with(b"<<"))
            .is_some_and(|o| {
                heredoc_literal(&s.parts().iter().collect::<Vec<_>>(), o.as_slice(), file)
            });
    }
    !matches!(
        node,
        Node::XStringNode { .. } | Node::InterpolatedXStringNode { .. }
    ) && (ast::is_literal(node) || ast::negated_numeric_literal(node).is_some())
}

// Port of the pinned Prism-to-parser compiler's static str token emission
// (MIT License, Copyright Shopify Inc.).
// Retain main-parse cooked bytes: source/payload part counts are NOT token counts.
fn emitted_strings(
    s: &ruby_prism::StringNode<'_>,
    opening: &[u8],
    file: &SourceFile,
) -> Vec<Vec<u8>> {
    emitted_literal(s.content_loc().as_slice(), s.unescaped(), opening, file)
}
fn emitted_literal(raw: &[u8], value: &[u8], opening: &[u8], file: &SourceFile) -> Vec<Vec<u8>> {
    let escaped = file.match_pattern_source(raw);
    let escaped: Vec<_> = escaped
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b != b'\r' || escaped.get(i + 1) != Some(&b'\n')).then_some(b))
        .collect();
    let single = opening.ends_with(b"'")
        || opening.starts_with(b"%q")
        || opening.starts_with(b"%s")
        || opening.starts_with(b"%w");
    let mut payload = ast::require_literal::parts(raw, value, single, file);
    if !single {
        super::lambda_without_literal_block::project_buffer_continuations(raw, &mut payload, file);
    }
    let payload: Vec<_> = payload.into_iter().flatten().collect();
    if !escaped.contains(&b'\n') {
        return vec![payload];
    }
    let cooked: Vec<_> = payload.split_inclusive(|&b| b == b'\n').collect();
    let percent_array = opening.starts_with(b"%w") || opening.starts_with(b"%W");
    if single {
        let mut result = Vec::new();
        let mut current = Vec::new();
        let mut lines = escaped
            .split_inclusive(|&b| b == b'\n')
            .enumerate()
            .peekable();
        while let Some((i, line)) = lines.next() {
            current.extend_from_slice(cooked.get(i).copied().unwrap_or_default());
            if percent_array
                && line.ends_with(b"\n")
                && line[..line.len() - 1]
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'\\')
                    .count()
                    % 2
                    == 1
                && lines.peek().is_some()
            {
                continue;
            }
            result.push(std::mem::take(&mut current));
        }
        return result;
    }
    let mut ends = Vec::new();
    let (mut slots, mut group_slots) = (0, 0);
    let mut lines = escaped.split_inclusive(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        let (mut backslashes, mut newlines) = (0, 0);
        for &b in line {
            if b == b'\\' {
                backslashes += 1;
            } else {
                newlines += usize::from(b == b'n' && backslashes % 2 == 1);
                backslashes = 0;
            }
        }
        if !line.ends_with(b"\n") && newlines > 0 {
            newlines -= 1;
        }
        group_slots += newlines + usize::from(percent_array);
        let before = line.strip_suffix(b"\n").unwrap_or(line);
        let before = before.strip_suffix(b"\r").unwrap_or(before);
        let continuation = line.ends_with(b"\n")
            && before.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
        if !continuation || lines.peek().is_none() {
            slots += group_slots + usize::from(!percent_array);
            ends.push(slots);
            group_slots = 0;
        }
    }
    let mut result = Vec::new();
    let mut current = Vec::new();
    for (i, line) in cooked.iter().enumerate() {
        current.extend_from_slice(line);
        if ends.binary_search(&(i + 1)).is_ok() {
            result.push(std::mem::take(&mut current));
        }
    }
    result
}
fn heredoc_literal(parts: &[Node<'_>], opening: &[u8], file: &SourceFile) -> bool {
    // An interpolation is dstr regardless of the value of its Ruby expression.
    // Do not fingerprint or investigate that expression for literal detection.
    if parts.iter().any(|p| p.as_string_node().is_none()) {
        return false;
    }
    matches!(
        composed_literal(parts, opening, "str", file, LiteralProfile::Scope),
        ScopeLiteral::Str(_)
    )
}

// A cop-specific literal AST key, used ONLY by the shared fingerprint's new
// conditional-scope mode. Existing literal identities/guards are untouched.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ScopeLiteral {
    Str(Vec<u8>),
    Sym(Vec<u8>),
    Compound(&'static str, Vec<ScopeLiteral>),
    Ruby(ast::Fingerprint<'static>),
    Complex(ComplexValue),
}
impl ScopeLiteral {
    pub(crate) fn same_ast_value(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Complex(a), Self::Complex(b)) => a.same_value(b),
            (Self::Ruby(a), Self::Ruby(b)) => a.conditional_scope_eq(b),
            (Self::Compound(a, left), Self::Compound(b, right)) => {
                a == b
                    && left.len() == right.len()
                    && left.iter().zip(right).all(|(a, b)| a.same_ast_value(b))
            }
            _ => self == other,
        }
    }
}

/// Complex components use Ruby numeric ==, unlike the distinct int/float/rational
/// AST node kinds. Retain exact ratios; converting all components to f64 would
/// silently equate different large integers or nearby rationals.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ComplexValue {
    exact: Option<(bool, Vec<u32>, Vec<u32>)>,
    rounded: u64,
}
impl ComplexValue {
    fn new(node: &Node<'_>) -> Self {
        let integer = |n: ruby_prism::Integer<'_>| {
            let (negative, digits) = n.to_u32_digits();
            let mut digits = digits.to_vec();
            while digits.last() == Some(&0) {
                digits.pop();
            }
            (negative && !digits.is_empty(), digits)
        };
        if let Some(n) = node.as_integer_node() {
            let (negative, digits) = integer(n.value());
            return Self {
                rounded: rounded_integer(negative, &digits).to_bits(),
                exact: Some((negative, digits, vec![1])),
            };
        }
        if let Some(n) = node.as_rational_node() {
            let (negative, numerator) = integer(n.numerator());
            let (_, denominator) = integer(n.denominator());
            // Rational literals are decimal or integral base-prefixed numerals.
            // Decimal parsing gives the correctly rounded value even when both
            // the exact numerator and denominator overflow f64 separately.
            let text = String::from_utf8_lossy(n.location().as_slice())
                .trim_end_matches('r')
                .replace('_', "");
            let rounded = if denominator == [1] {
                rounded_integer(negative, &numerator)
            } else {
                text.parse::<f64>().expect("decimal rational literal")
            };
            return Self {
                exact: Some((negative, numerator, denominator)),
                rounded: rounded.to_bits(),
            };
        }
        Self {
            exact: None,
            rounded: node.as_float_node().unwrap().value().to_bits(),
        }
    }
    fn same_value(&self, other: &Self) -> bool {
        match (&self.exact, &other.exact) {
            (Some(a), Some(b)) => a == b,
            // Complex's f_eqeq_p converts BOTH components to double if either
            // is Float, unlike ordinary Integer#== Float. This also means its
            // equality is not transitive; never use this as a Hash/Eq key.
            _ => f64::from_bits(self.rounded) == f64::from_bits(other.rounded),
        }
    }
}
fn rounded_integer(negative: bool, digits: &[u32]) -> f64 {
    let Some(last) = digits.last() else {
        return 0.0;
    };
    let bits = (digits.len() - 1) * 32 + (32 - last.leading_zeros()) as usize;
    let shift = bits.saturating_sub(53);
    let bit = |i: usize| digits[i / 32] & (1 << (i % 32)) != 0;
    let mut mantissa = (shift..bits)
        .rev()
        .fold(0_u64, |n, i| (n << 1) | u64::from(bit(i)));
    if shift > 0 && bit(shift - 1) && (mantissa % 2 == 1 || (0..shift - 1).any(bit)) {
        mantissa += 1;
    }
    let value = mantissa as f64 * 2_f64.powi(shift.min(i32::MAX as usize) as i32);
    if negative { -value } else { value }
}
fn compose(kind: &'static str, mut parts: Vec<ScopeLiteral>) -> ScopeLiteral {
    if kind != "xstr"
        && parts.len() == 1
        && let ScopeLiteral::Str(_) = &parts[0]
    {
        let ScopeLiteral::Str(value) = parts.pop().unwrap() else {
            unreachable!()
        };
        return if kind == "sym" {
            ScopeLiteral::Sym(value)
        } else {
            ScopeLiteral::Str(value)
        };
    }
    ScopeLiteral::Compound(
        match kind {
            "str" => "dstr",
            "sym" => "dsym",
            _ => kind,
        },
        parts,
    )
}
#[derive(Clone, Copy)]
enum LiteralProfile<'a> {
    Scope,
    Duplicate(ast::DuplicateEquality),
    Gemspec(&'a std::collections::HashSet<(usize, usize)>),
    SelfAssignment(Option<&'a std::collections::HashSet<(usize, usize)>>),
}

fn composed_literal(
    parts: &[Node<'_>],
    opening: &[u8],
    kind: &'static str,
    file: &SourceFile,
    profile: LiteralProfile<'_>,
) -> ScopeLiteral {
    let heredoc = opening.starts_with(b"<<");
    let mut children = Vec::new();
    for part in parts {
        let items = if let Some(s) = part.as_string_node() {
            // Concatenated complete quoted literals are separate AST children;
            // interpolation fragments instead emit physical str tokens directly.
            if s.opening_loc().is_some() && !heredoc {
                vec![literal_with_profile(part, file, profile).unwrap()]
            } else {
                emitted_strings(&s, opening, file)
                    .into_iter()
                    .map(ScopeLiteral::Str)
                    .collect()
            }
        } else {
            vec![
                literal_with_profile(part, file, profile)
                    .unwrap_or_else(|| ScopeLiteral::Ruby(embedded_ruby(part, file, profile))),
            ]
        };
        for item in items {
            if heredoc && let ScopeLiteral::Str(value) = &item {
                if value.is_empty() {
                    continue;
                }
                if let Some(ScopeLiteral::Str(previous)) = children.last_mut()
                    && !previous.ends_with(b"\n")
                {
                    previous.extend(value);
                    continue;
                }
            }
            children.push(item);
        }
    }
    compose(kind, children)
}
// Buffer CRLF normalization can join two main-parse percent-array words:
// a \\ CR CR LF becomes the escaped CRLF of one word, retaining a cooked LF.
// Join only the proven main-parse edge; no rescanning Ruby grammar or reparsing.
fn percent_words(
    a: &ruby_prism::ArrayNode<'_>,
    opening: &[u8],
    file: &SourceFile,
    profile: LiteralProfile<'_>,
) -> Vec<ScopeLiteral> {
    enum Part {
        Text(usize, usize, Vec<u8>),
        Ruby(ast::Fingerprint<'static>),
    }
    let text = |loc: Location<'_>, value: &[u8]| {
        Part::Text(loc.start_offset(), loc.end_offset(), value.to_vec())
    };
    let symbols = opening.starts_with(b"%i") || opening.starts_with(b"%I");
    let mut words: Vec<(Vec<Part>, bool)> = Vec::new();
    let mut previous_end = None;
    let mut joins_next = false;
    for n in a.elements().iter() {
        let loc = n.location();
        let dynamic =
            n.as_interpolated_string_node().is_some() || n.as_interpolated_symbol_node().is_some();
        let mut parts = if let Some(s) = n.as_string_node() {
            vec![text(s.content_loc(), s.unescaped())]
        } else if let Some(s) = n.as_symbol_node() {
            vec![text(
                s.value_loc().unwrap_or_else(|| s.location()),
                s.unescaped(),
            )]
        } else {
            let children = if let Some(s) = n.as_interpolated_string_node() {
                s.parts()
            } else {
                n.as_interpolated_symbol_node().unwrap().parts()
            };
            children
                .iter()
                .map(|p| {
                    if let Some(s) = p.as_string_node() {
                        text(s.content_loc(), s.unescaped())
                    } else {
                        Part::Ruby(embedded_ruby(&p, file, profile))
                    }
                })
                .collect()
        };
        if joins_next
            && previous_end.is_some_and(|end| &file.bytes[end..loc.start_offset()] == b"\r\n")
        {
            let (previous, was_dynamic) = words.last_mut().unwrap();
            *was_dynamic |= dynamic;
            if let Some(Part::Text(_, end, value)) = previous.last_mut() {
                if value.ends_with(b"\r") {
                    value.pop();
                    value.push(b'\n');
                }
                *end = loc.start_offset();
                if matches!(parts.first(), Some(Part::Text(..))) {
                    let Part::Text(_, next_end, next_value) = parts.remove(0) else {
                        unreachable!()
                    };
                    *end = next_end;
                    value.extend(next_value);
                }
            }
            previous.extend(parts);
        } else {
            words.push((parts, dynamic));
        }
        let raw = loc.as_slice();
        joins_next = raw.ends_with(b"\r")
            && raw[..raw.len() - 1]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count()
                % 2
                == 1;
        previous_end = Some(loc.end_offset());
    }
    words
        .into_iter()
        .flat_map(|(parts, dynamic)| {
            let mut children = Vec::new();
            for part in parts {
                match part {
                    Part::Ruby(key) => children.push(ScopeLiteral::Ruby(key)),
                    Part::Text(start, end, value) => {
                        let raw = &file.bytes[start..end];
                        if symbols && !dynamic {
                            // Static word symbols bypass string token emission entirely.
                            let value = ast::require_literal::parts(
                                raw,
                                &value,
                                opening.starts_with(b"%i"),
                                file,
                            )
                            .into_iter()
                            .flatten()
                            .collect();
                            children.push(ScopeLiteral::Str(value));
                        } else {
                            // Interpolated word symbols are visited independently by the
                            // compiler, with no percent-array opening (unlike %W strings).
                            let context = if symbols { b"".as_slice() } else { opening };
                            children.extend(
                                emitted_literal(raw, &value, context, file)
                                    .into_iter()
                                    .map(ScopeLiteral::Str),
                            );
                        }
                    }
                }
            }
            if symbols {
                vec![compose("sym", children)]
            } else if dynamic {
                vec![compose("str", children)]
            } else {
                children
            }
        })
        .collect()
}
fn embedded_ruby(
    node: &Node<'_>,
    file: &SourceFile,
    profile: LiteralProfile<'_>,
) -> ast::Fingerprint<'static> {
    let key = match profile {
        LiteralProfile::Scope => ast::fingerprint_for_conditional_scope(node, file),
        LiteralProfile::Duplicate(ast::DuplicateEquality::Hash) => {
            ast::fingerprint_for_duplicate(node, file)
        }
        LiteralProfile::Duplicate(ast::DuplicateEquality::Value) => {
            ast::fingerprint_for_duplicate_condition(node, file)
        }
        LiteralProfile::SelfAssignment(locals) => {
            ast::fingerprint_for_self_assignment(node, file, locals)
        }
        LiteralProfile::Gemspec(locals) => ast::fingerprint_for_gemspec_key(node, file, locals),
    };
    key.into_owned()
}

pub(crate) fn scope_literal(node: &Node<'_>, file: &SourceFile) -> Option<ScopeLiteral> {
    scope_literal_with_profile(node, file, None)
}

/// Duplicate-only profiles propagate through every compound/embedded literal.
/// The ordinary conditional-scope projection is deliberately unchanged.
pub(crate) fn scope_literal_with_profile(
    node: &Node<'_>,
    file: &SourceFile,
    profile: Option<ast::DuplicateEquality>,
) -> Option<ScopeLiteral> {
    literal_with_profile(
        node,
        file,
        profile.map_or(LiteralProfile::Scope, LiteralProfile::Duplicate),
    )
}

/// Preserve exact literal emission while carrying the Gemspec-only AST profile
/// through embedded Ruby, including grouping and cached local/source facts.
/// Other literal/fingerprint/correction consumers keep their existing profile.
pub(crate) fn gemspec_literal(
    node: &Node<'_>,
    file: &SourceFile,
    locals: &std::collections::HashSet<(usize, usize)>,
) -> Option<ScopeLiteral> {
    literal_with_profile(node, file, LiteralProfile::Gemspec(locals))
}

/// Reuse exact main-parse literal/source-character alignment, carrying
/// SelfAssignment's detection-only parser AST equality into embedded Ruby.
pub(crate) fn self_assignment_literal(
    node: &Node<'_>,
    file: &SourceFile,
    locals: Option<&std::collections::HashSet<(usize, usize)>>,
) -> Option<ScopeLiteral> {
    literal_with_profile(node, file, LiteralProfile::SelfAssignment(locals))
}

fn literal_with_profile(
    node: &Node<'_>,
    file: &SourceFile,
    profile: LiteralProfile<'_>,
) -> Option<ScopeLiteral> {
    if let Some(n) = node.as_imaginary_node() {
        return Some(
            if matches!(
                profile,
                LiteralProfile::Duplicate(ast::DuplicateEquality::Hash)
                    | LiteralProfile::Gemspec(_)
            ) {
                // A typed numeric child gives Complex#eql? identity: int/rational/
                // float stay distinct, exact ratios reduce, floating signed zeros
                // agree. Do not replace Elsif's non-transitive Complex#== key.
                ScopeLiteral::Ruby(ast::fingerprint_for_detection(node, file).into_owned())
            } else {
                ScopeLiteral::Complex(ComplexValue::new(&n.numeric()))
            },
        );
    }
    if let Some(a) = node.as_array_node() {
        let opening = a.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
        if [b"%w".as_slice(), b"%W", b"%i", b"%I"]
            .iter()
            .any(|p| opening.starts_with(p))
        {
            return Some(ScopeLiteral::Compound(
                "array",
                percent_words(&a, opening, file, profile),
            ));
        }
        let children = a
            .elements()
            .iter()
            .map(|n| {
                literal_with_profile(&n, file, profile)
                    .unwrap_or_else(|| ScopeLiteral::Ruby(embedded_ruby(&n, file, profile)))
            })
            .collect();
        return Some(ScopeLiteral::Compound("array", children));
    }
    if node.as_source_file_node().is_some() {
        return Some(ScopeLiteral::Str(
            file.source_path().as_os_str().as_encoded_bytes().to_vec(),
        ));
    }
    if let Some(s) = node.as_string_node() {
        let opening = s.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
        if opening.starts_with(b"<<") {
            return Some(composed_literal(
                &[s.as_node()],
                opening,
                "str",
                file,
                profile,
            ));
        }
        let parts = if opening.starts_with(b"%") && s.unescaped().is_empty() {
            Vec::new()
        } else {
            emitted_strings(&s, opening, file)
                .into_iter()
                .map(ScopeLiteral::Str)
                .collect()
        };
        return Some(compose("str", parts));
    }
    if let Some(s) = node.as_symbol_node() {
        if s.closing_loc().is_none() {
            return Some(ScopeLiteral::Sym(
                file.match_pattern_source(s.unescaped()).into_owned(),
            ));
        }
        let opening = s.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
        let raw = s.value_loc();
        let raw = raw.as_ref().map_or(b"".as_slice(), |r| r.as_slice());
        if s.closing_loc()
            .is_some_and(|c| c.as_slice().ends_with(b":"))
        {
            // Assoc's pair_quoted bypasses ordinary symbol composition: a
            // static label always supplies ONE string child, even for empty
            // or multiline content. Project source glyphs before escapes as
            // usual, but never turn the label into a zero/multi-child dsym.
            return Some(ScopeLiteral::Sym(
                ast::require_literal::parts(raw, s.unescaped(), opening.ends_with(b"'"), file)
                    .concat(),
            ));
        }
        let parts = if raw.is_empty() {
            Vec::new()
        } else {
            emitted_literal(raw, s.unescaped(), opening, file)
                .into_iter()
                .map(ScopeLiteral::Str)
                .collect()
        };
        return Some(compose("sym", parts));
    }
    if let Some(s) = node.as_x_string_node() {
        let opening = s.opening_loc();
        let parts = if s.content_loc().as_slice().is_empty() {
            Vec::new()
        } else {
            emitted_literal(
                s.content_loc().as_slice(),
                s.unescaped(),
                opening.as_slice(),
                file,
            )
            .into_iter()
            .map(ScopeLiteral::Str)
            .collect()
        };
        return Some(compose("xstr", parts));
    }
    let (parts, opening, kind) = if let Some(s) = node.as_interpolated_string_node() {
        (s.parts(), s.opening_loc(), "str")
    } else if let Some(s) = node.as_interpolated_symbol_node() {
        (s.parts(), s.opening_loc(), "sym")
    } else {
        let s = node.as_interpolated_x_string_node()?;
        (s.parts(), Some(s.opening_loc()), "xstr")
    };
    Some(composed_literal(
        &parts.iter().collect::<Vec<_>>(),
        opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice()),
        kind,
        file,
        profile,
    ))
}

fn in_condition(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let mut child = node;
    let mut ancestors = cx.ancestors().peekable();
    while let Some(parent) = ancestors.next() {
        let predicate = if let Some(n) = parent.as_if_node() {
            Some(n.predicate())
        } else if let Some(n) = parent.as_unless_node() {
            Some(n.predicate())
        } else if let Some(n) = parent.as_while_node() {
            (!n.is_begin_modifier()).then(|| n.predicate())
        } else {
            parent
                .as_until_node()
                .filter(|n| !n.is_begin_modifier())
                .map(|n| n.predicate())
        };
        if predicate.is_some_and(|p| same_location(&p, child))
            && !ancestors.peek().is_some_and(|p| p.as_in_node().is_some())
        {
            return true;
        }
        if scope_body(parent, child, file) {
            return false;
        }
        child = parent;
    }
    false
}
/// Each scope/child edge is fingerprinted at most once, even when thousands of
/// assignments share its args or receiver. This is not binding/type-flow analysis.
#[derive(Default)]
pub(crate) struct ScopeEdges(std::collections::HashMap<(usize, usize, usize, usize), bool>);
// Prism owns a literal block under send, super, or zsuper; the pinned AST
// instead wraps that callee in any_block. A block pass is NOT such a wrapper.
fn literal_block<'pr>(node: &Node<'pr>) -> Option<ruby_prism::BlockNode<'pr>> {
    let block = match node {
        Node::CallNode { .. } => node.as_call_node()?.block(),
        Node::SuperNode { .. } => node.as_super_node()?.block(),
        Node::ForwardingSuperNode { .. } => return node.as_forwarding_super_node()?.block(),
        _ => None,
    };
    block.and_then(|b| b.as_block_node())
}
fn scope_body(parent: &Node<'_>, child: &Node<'_>, file: &SourceFile) -> bool {
    if !matches!(
        parent,
        Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
    ) && literal_block(parent).is_none()
    {
        return false;
    }
    let p = parent.location();
    let c = child.location();
    let key = (
        p.start_offset(),
        p.end_offset(),
        c.start_offset(),
        c.end_offset(),
    );
    let mut edges = file.literal_assignment_scope_edges().lock().unwrap();
    *edges
        .0
        .entry(key)
        .or_insert_with(|| compute_scope_body(parent, child, file))
}
fn compute_scope_body(parent: &Node<'_>, child: &Node<'_>, file: &SourceFile) -> bool {
    let body = parent
        .as_block_node()
        .and_then(|n| n.body())
        .or_else(|| parent.as_lambda_node().and_then(|n| n.body()))
        .or_else(|| parent.as_def_node().and_then(|n| n.body()));
    if let Some(body) = body {
        // Receiver grouping parentheses are removed by the pinned defs builder.
        let candidate = if parent
            .as_def_node()
            .and_then(|d| d.receiver())
            .is_some_and(|r| same_location(&r, child))
        {
            child.as_parentheses_node().and_then(|p| p.body())
        } else {
            None
        };
        return same_location(&body, child)
            || ast::fingerprint_for_conditional_scope(&body, file).conditional_scope_eq(
                &ast::fingerprint_for_conditional_scope(candidate.as_ref().unwrap_or(child), file),
            );
    }
    if let Some(call) = parent.as_super_node()
        && let Some(block) = literal_block(parent)
    {
        if child.as_block_node().is_some() {
            return false;
        }
        // Compare the synthetic :super callee (without its literal block),
        // NOT its individual argument child. Keep :zsuper distinct, grouping
        // and multiple statements intact, and reject a body with its own block
        // or &block pass. Argument keys retain keyword/splat/forwarding shapes.
        if let Some(body) = block
            .body()
            .and_then(|b| b.as_statements_node())
            .filter(|s| s.body().len() == 1)
            .and_then(|s| s.body().iter().next())
            .and_then(|n| n.as_super_node())
        {
            let left = call
                .arguments()
                .map(|a| a.arguments().iter().collect::<Vec<_>>())
                .unwrap_or_default();
            let right = body
                .arguments()
                .map(|a| a.arguments().iter().collect::<Vec<_>>())
                .unwrap_or_default();
            return body.block().is_none()
                && left.len() == right.len()
                && left.iter().zip(&right).all(|(a, b)| {
                    ast::fingerprint_for_conditional_scope(a, file)
                        .conditional_scope_eq(&ast::fingerprint_for_conditional_scope(b, file))
                });
        }
    }
    // ForwardingSuperNode (:zsuper) has only a block child, so there is no
    // argument/receiver callee edge to skip. Its block parameters still climb
    // normally, and its body stops at the existing BlockNode boundary.
    if let Some(call) = parent.as_call_node()
        && let Some(block) = literal_block(parent)
    {
        if child.as_block_node().is_some() {
            return false;
        }
        // RuboCop wraps send,args,body in any_block; its send has NO block child.
        if let Some(body) = block
            .body()
            .and_then(|b| b.as_statements_node())
            .filter(|s| s.body().len() == 1)
            .and_then(|s| s.body().iter().next())
            .and_then(|n| n.as_call_node())
        {
            let left = ast::call_arguments(&call);
            let right = ast::call_arguments(&body);
            return body.block().is_none()
                && call.name().as_slice() == body.name().as_slice()
                && call.is_safe_navigation() == body.is_safe_navigation()
                && match (call.receiver(), body.receiver()) {
                    (None, None) => true,
                    (Some(a), Some(b)) => ast::fingerprint_for_conditional_scope(&a, file)
                        .conditional_scope_eq(&ast::fingerprint_for_conditional_scope(&b, file)),
                    _ => false,
                }
                && left.len() == right.len()
                && left.iter().zip(right.iter()).all(|(a, b)| {
                    ast::fingerprint_for_conditional_scope(a, file)
                        .conditional_scope_eq(&ast::fingerprint_for_conditional_scope(b, file))
                });
        }
    }
    false
}
fn same_location(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}

/// LiteralAssignmentInCondition's bounded, lazy structural scope-edge facts.
pub(crate) static LITERAL_ASSIGNMENT_SCOPE_EDGES: crate::source::Fact<
    std::sync::Mutex<ScopeEdges>,
> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn literal_assignment_scope_edges(&self) -> &std::sync::Mutex<ScopeEdges> {
        LITERAL_ASSIGNMENT_SCOPE_EDGES.get(self, Default::default)
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/literal_assignment_in_condition_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/LiteralAssignmentInCondition")
            .collect()
    }
    #[test]
    fn static_quoted_labels_have_one_symbol_payload_in_every_detection_profile() {
        for (body, expected) in [
            (r#"{"": 1}"#, ""),
            (r#"{'': 1}"#, ""),
            (r#"{"\u{}": 1}"#, ""),
            ("{\"a\nb\": 1}", "a\nb"),
            (r#"{"é": 1}"#, "é"),
            ("{\"e\u{301}\": 1}", "é"),
            (r#"{"\u0065\u0301": 1}"#, "e\u{301}"),
        ] {
            let source = format!("# coding: UTF8-MAC\n{body}\n");
            let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{body}");
            let hash = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .iter()
                .next()
                .unwrap();
            let pair = hash
                .as_hash_node()
                .unwrap()
                .elements()
                .iter()
                .next()
                .unwrap();
            let key = pair.as_assoc_node().unwrap().key();
            for profile in [
                LiteralProfile::Scope,
                LiteralProfile::Duplicate(ast::DuplicateEquality::Hash),
                LiteralProfile::Duplicate(ast::DuplicateEquality::Value),
                LiteralProfile::SelfAssignment(None),
            ] {
                assert_eq!(
                    literal_with_profile(&key, &file, profile),
                    Some(ScopeLiteral::Sym(expected.as_bytes().to_vec())),
                    "{body}"
                );
            }
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes_and_literal_collections() {
        for rhs in ["42", "'foo'", "[]", "[1, 2, 3]", "{}", "{x: :y}"] {
            let found = offenses(&format!("if test = {rhs}\nend\n"));
            assert_eq!(found.len(), 1, "{rhs}");
            assert_eq!((found[0].line, found[0].col), (1, 9));
            assert_eq!(
                found[0].message,
                format!(
                    "Don't use literal assignment `= {rhs}` in conditional, should be `==` or non-literal operand."
                )
            );
        }
        for source in [
            "if test = \"#{foo}\"; end",
            "if test = `echo 'hi'`; end",
            "if test = [42, x, y]; end",
            "if test = {x => :y}; end",
            "if test = {x: y}; end",
            "if test = do_something; end",
            "if collection[index] = 42; end",
            "if test.method = 42; end",
            "if test == 10; end",
            "raise StandardError unless foo ||= bar",
            "if (); end",
            "unless (); end",
            "foo { x if y = z }",
            "return if do_something do |z|; x, y = *z; end",
            "return 1 if any_errors? { o = inspect(file) }",
            "return 1 if any_errors? { o = file }.present?",
            "if x?(bar) || y? { z = baz }; foo; end",
            "raise StandardError unless (foo ||= bar) || a = b",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_assignment_kinds_and_conditional_forms() {
        for (source, col) in [
            ("while test = 42; end", 12),
            ("until test = 42; end", 12),
            ("if @test = 42; end", 10),
            ("if @@test = 42; end", 11),
            ("if $test = 42; end", 10),
            ("if TEST = 42; end", 9),
            ("if test == 10 || foo = 1; end", 22),
            ("foo { x if y = 42 }", 14),
            ("raise StandardError unless (foo ||= bar) || a = 42", 47),
            ("if (test = 42); end", 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
        }
        for source in [
            "a if x = 1",
            "a unless x = 1",
            "(x = 1) ? a : b",
            "a while x = 1",
            "a until x = 1",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "begin; a; end while x = 1",
            "begin; a; end until x = 1",
            "case a; in b if x = 1; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn scope_bodies_are_skipped_but_independent_conditions_and_classes_are_not() {
        for source in [
            "if foo { x = 1 }; end",
            "if -> { x = 1 }; end",
            "if def f; x = 1; end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "if foo { x if y = 1 }; end",
            "if def f(x = (y = 1)); end; end",
            "if class C; x = 1; end; end",
            "if class << self; x = 1; end; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("if foo(x = 1) { foo(x = 0x1) }; end").is_empty());
        assert!(offenses("if def (x = 1).f; x = 1; end; end").is_empty());
        assert_eq!(offenses("if foo(x = 1) { (foo(x = 1)) }; end").len(), 1);
    }
    #[test]
    fn super_literal_block_skips_only_the_structurally_equal_callee_edge() {
        for (body, equal) in [
            ("super(x = 1)", true),
            ("super(x = 0x1)", true),
            ("super(x = 2)", false),
            ("(super(x = 1))", false),
            ("super(x = 1); nil", false),
            ("super(x = 1) {}", false),
        ] {
            for params in ["", "|a = (y = 1)|"] {
                let source = format!("def f; if super(x = 1) {{ {params} {body} }}; end; end\n");
                let found = offenses(&source);
                let mut expected = Vec::new();
                if !equal {
                    expected.push((1, 19));
                }
                if !params.is_empty() {
                    expected.push((1, 34));
                }
                assert_eq!(
                    found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                    expected,
                    "{source}"
                );
                for offense in found {
                    assert_eq!(offense.end_line, 1);
                    assert_eq!(
                        offense.message,
                        "Don't use literal assignment `= 1` in conditional, should be `==` or non-literal operand."
                    );
                    assert!(offense.correction.is_none());
                }
            }
        }
    }
    #[test]
    fn complex_scope_values_match_across_numeric_kinds_without_rounding_exact_values() {
        for (left, right, equal) in [
            ("1i", "1.0i", true),
            ("1i", "1ri", true),
            ("1.0i", "1.0ri", true),
            ("0x10i", "16.0i", true),
            ("-1i", "-1.0ri", true),
            ("0i", "-0.0i", true),
            ("0.1ri", "0.1i", true),
            ("010ri", "8.0i", true),
            ("0x20000000000001ri", "9007199254740992.0i", true),
            ("0x20000000000003ri", "9007199254740996.0i", true),
            ("0.10000000000000001ri", "0.1i", true),
            ("1i", "2.0i", false),
            ("-1i", "1.0i", false),
            ("9007199254740993i", "9007199254740992.0i", true),
            ("9007199254740994i", "9007199254740992.0i", false),
            ("9007199254740993i", "9007199254740992i", false),
            ("9007199254740993ri", "9007199254740992ri", false),
            ("0.10000000000000001ri", "0.1ri", false),
        ] {
            for (left, right) in [(left, right), (right, left)] {
                for source in [
                    format!("if foo(x = 1, {left}) {{ foo(x = 1, {right}) }}; end"),
                    format!("if def (foo(x = 1, {left})).f; foo(x = 1, {right}); end; end"),
                    format!("if foo(x = [{left}]) {{ foo(x = [{right}]) }}; end"),
                ] {
                    let found = offenses(&source);
                    assert_eq!(found.len(), usize::from(!equal), "{source}");
                    if !equal {
                        assert_eq!(found[0].line, 1);
                        assert!(found[0].message.starts_with("Don't use literal assignment"));
                    }
                }
            }
        }
    }
    #[test]
    fn exact_static_string_tokens_not_decoded_truthiness() {
        for source in [
            "if x = \"a\nb\"; end",
            "if x = %Q(); end",
            "if x = \"\\\n\"; end",
            "if x = <<TEXT;end\nTEXT\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "if x = \"\n\"; end",
            "if x = \"a\\\nb\"; end",
            "if x = {**dynamic}; end",
            "if x = /#{dynamic}/;end",
            "if x = dynamic..other;end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert_eq!(
            offenses("if foo(x = 1, \"a\nb\") { foo(x = 1, \"a\\nb\") }; end").len(),
            1
        );
    }
    #[test]
    fn exact_scope_literal_keys_do_not_change_any_existing_fingerprint_mode() {
        let file = SourceFile::new("example.rb".into(), b"foo(\"a\nb\", \"a\\nb\")".to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        file.record_encoding(&parsed);
        let program = parsed.node().as_program_node().unwrap();
        let call = program.statements().body().iter().next().unwrap();
        let args = ast::call_arguments(&call.as_call_node().unwrap());
        assert_eq!(ast::fingerprint(&args[0]), ast::fingerprint(&args[1]));
        assert_eq!(
            ast::fingerprint_for_branch(&args[0], &file),
            ast::fingerprint_for_branch(&args[1], &file)
        );
        assert_ne!(
            ast::fingerprint_for_conditional_scope(&args[0], &file),
            ast::fingerprint_for_conditional_scope(&args[1], &file)
        );
    }
    #[test]
    fn complex_numeric_equality_is_confined_to_conditional_scope_comparisons() {
        let source = b"foo(1i, 1.0i, 1ri)";
        let file = SourceFile::new("example.rb".into(), source.to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        let program = parsed.node().as_program_node().unwrap();
        let call = program.statements().body().iter().next().unwrap();
        let args = ast::call_arguments(&call.as_call_node().unwrap());
        for right in &args[1..] {
            assert!(
                ast::fingerprint_for_conditional_scope(&args[0], &file)
                    .conditional_scope_eq(&ast::fingerprint_for_conditional_scope(right, &file))
            );
            assert_ne!(ast::fingerprint(&args[0]), ast::fingerprint(right));
            assert_ne!(
                ast::fingerprint_for_branch(&args[0], &file),
                ast::fingerprint_for_branch(right, &file)
            );
            assert_ne!(
                ast::fingerprint_for_comparison(&args[0], &file),
                ast::fingerprint_for_comparison(right, &file)
            );
            assert_ne!(
                ast::fingerprint_ignoring_grouping(&args[0], &file.bytes),
                ast::fingerprint_ignoring_grouping(right, &file.bytes)
            );
        }
    }
    #[test]
    fn many_assignments_share_two_cached_scope_edges_without_predicate_rescans() {
        let args = (0..1000)
            .map(|i| format!("x{i} = 1"))
            .collect::<Vec<_>>()
            .join(", ");
        let body = (0..1000)
            .map(|i| format!("x{i} = 2"))
            .collect::<Vec<_>>()
            .join(", ");
        let file = SourceFile::new(
            "example.rb".into(),
            format!("if foo({args}) {{ foo({body}) }};end\n").into_bytes(),
        );
        let parsed = crate::parser::parse(&file.bytes);
        file.record_encoding(&parsed);
        let mut found = Vec::new();
        assert!(
            crate::cops::run_cops(&[&LiteralAssignmentInCondition], &parsed, &file, &mut found)
                .is_empty()
        );
        assert_eq!(found.len(), 1000);
        assert_eq!(
            file.literal_assignment_scope_edges()
                .lock()
                .unwrap()
                .0
                .len(),
            2
        );
    }
    #[test]
    fn super_assignments_share_two_cached_edges_and_forwarding_keeps_parameters() {
        for callee in ["super", "super()"] {
            let source = format!("def f; if {callee} {{ |a = (y = 1)| x = 1 }}; end; end");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].line, 1);
            assert_eq!(found[0].col as usize, source.find("= 1").unwrap() + 1);
            assert!(found[0].correction.is_none());
        }
        let args = (0..1000)
            .map(|i| format!("x{i} = 1"))
            .collect::<Vec<_>>()
            .join(", ");
        let file = SourceFile::new(
            "example.rb".into(),
            format!("def f; if super({args}) {{ super({args}) }}; end; end\n").into_bytes(),
        );
        let parsed = crate::parser::parse(&file.bytes);
        file.record_encoding(&parsed);
        let mut found = Vec::new();
        assert!(
            crate::cops::run_cops(&[&LiteralAssignmentInCondition], &parsed, &file, &mut found)
                .is_empty()
        );
        assert!(found.is_empty());
        assert_eq!(
            file.literal_assignment_scope_edges()
                .lock()
                .unwrap()
                .0
                .len(),
            2
        );
    }
    #[test]
    fn pinned_scalar_assignment_reports_operator_and_literal() {
        let found = offenses("if test = 42\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 9, 1));
        assert_eq!(
            found[0].message,
            "Don't use literal assignment `= 42` in conditional, should be `==` or non-literal operand."
        );
        assert!(found[0].correction.is_none());
    }
}
