//! Bounded syntax-equivalence verification: a port of the scope grouping
//! and batch verification of RuboCop's `ReparsedEquivalence` mixin
//! (97f30c4, MIT), extracted from `Style/RedundantParentheses` (its first
//! consumer) for `Layout/RedundantLineBreak`.
//!
//! Candidate corrections are grouped by their reparse scope, the innermost
//! `def`, `class`, `module` or `class << x` holding them (these neither
//! capture outer locals nor continue an outer expression); only that
//! fragment is parsed again, the whole file outside any scope. Each group is
//! verified in one batch: the original fragment and the corrected one are
//! parsed with the main parser (its fixed Ruby 3.4 grammar, resource and
//! depth limits), optionally after a preprocessing step applied to both, and
//! their fingerprints compared. A failing batch is split in halves, so a few
//! failing candidates cost a logarithmic number of reparses. A fragment that
//! does not parse alone verifies nothing, as in RuboCop. This is syntax
//! inspection only: nothing is evaluated.
use std::{borrow::Cow, collections::BTreeMap, ops::Range};

use ruby_prism::Node;

use super::{
    ast::Fingerprint,
    walk::{NodeVisitor, walk},
};
use crate::source::SourceFile;

/// How a consumer compares trees: the fingerprint of a parsed fragment
/// (given the parsed bytes) and a rewrite applied to both sides first.
pub(crate) struct Equivalence {
    pub fingerprint: for<'pr> fn(&Node<'pr>, &'pr [u8]) -> Fingerprint<'pr>,
    pub preprocess: for<'a> fn(&'a [u8]) -> Cow<'a, [u8]>,
}

/// Applies the corrections of the candidates `group` (indices) to `source`,
/// a fragment starting at offset `start` of the file; `None` when they
/// clash.
pub(crate) type Apply<'a> = dyn Fn(&[u8], usize, &[usize]) -> Option<Vec<u8>> + 'a;

/// The indices of the candidates whose corrections leave the parse
/// unchanged under `equivalence`. `spans[i]` is the byte range candidate
/// `i`'s edits touch.
pub(crate) fn verified(
    file: &SourceFile,
    program: &Node<'_>,
    spans: &[Range<usize>],
    apply: &Apply<'_>,
    equivalence: &Equivalence,
) -> Vec<usize> {
    if spans.is_empty() {
        return Vec::new();
    }
    Verifier::new(program, equivalence).verified(file, program, spans, apply)
}

/// Verification prepared once per file for a consumer that verifies its
/// candidates one call at a time: the reparse scopes are collected and each
/// scope's original fingerprint is computed once, not on every call. Every
/// call still parses its corrected fragment.
pub(crate) struct Verifier<'e> {
    equivalence: &'e Equivalence,
    scopes: Vec<Range<usize>>,
    /// The original fingerprint by scope (`None`: the whole file); `None`
    /// when the original fragment does not parse alone.
    expected: BTreeMap<Option<(usize, usize)>, Option<Fingerprint<'static>>>,
}

impl<'e> Verifier<'e> {
    pub(crate) fn new(program: &Node<'_>, equivalence: &'e Equivalence) -> Self {
        Self {
            equivalence,
            scopes: reparse_scopes(program),
            expected: BTreeMap::new(),
        }
    }

    /// [`verified`] with this file's prepared scopes and fingerprints.
    pub(crate) fn verified(
        &mut self,
        file: &SourceFile,
        program: &Node<'_>,
        spans: &[Range<usize>],
        apply: &Apply<'_>,
    ) -> Vec<usize> {
        let equivalence = self.equivalence;
        let mut groups: BTreeMap<Option<(usize, usize)>, Vec<usize>> = BTreeMap::new();
        for (index, span) in spans.iter().enumerate() {
            let scope = innermost_scope(&self.scopes, span).map(|scope| (scope.start, scope.end));
            groups.entry(scope).or_default().push(index);
        }
        let mut accepted = Vec::new();
        for (scope, group) in groups {
            let (start, end) = scope.unwrap_or((0, file.bytes.len()));
            let fragment = &file.bytes[start..end];
            let expected = self.expected.entry(scope).or_insert_with(|| {
                let original = (equivalence.preprocess)(fragment);
                if scope.is_none() && matches!(original, Cow::Borrowed(_)) {
                    // The whole unchanged file: the main parse already has
                    // its tree.
                    return Some((equivalence.fingerprint)(program, &file.bytes).into_owned());
                }
                let parsed = crate::parser::parse_as(&original, file.ruby_version());
                if parsed.errors().next().is_some() || parsed.depth_error().is_some() {
                    // RuboCop cannot verify a fragment that does not parse
                    // alone either (a heredoc body past an endless def).
                    return None;
                }
                Some((equivalence.fingerprint)(&parsed.node(), &original).into_owned())
            });
            let Some(expected) = expected else {
                continue;
            };
            accepted.extend(verified_subset(
                file.ruby_version(),
                fragment,
                start,
                expected,
                apply,
                equivalence,
                group,
            ));
        }
        accepted.sort_unstable();
        accepted
    }
}

/// The innermost scope holding `span`. Scopes nest or are disjoint and are
/// sorted by start: it is the last one starting at or before the span that
/// holds it.
fn innermost_scope<'a>(
    scopes: &'a [Range<usize>],
    span: &Range<usize>,
) -> Option<&'a Range<usize>> {
    let after = scopes.partition_point(|scope| scope.start <= span.start);
    scopes[..after]
        .iter()
        .rev()
        .find(|scope| scope.start <= span.start && span.end <= scope.end)
}

/// The indices in `group` whose corrections, applied to `fragment`, parse
/// to the `expected` fingerprint: all at once, or by halves while a batch
/// fails.
fn verified_subset(
    version: crate::cops::policy::RubyVersion,
    fragment: &[u8],
    start: usize,
    expected: &Fingerprint<'_>,
    apply: &Apply<'_>,
    equivalence: &Equivalence,
    group: Vec<usize>,
) -> Vec<usize> {
    let mut accepted = Vec::new();
    let mut pending = vec![group];
    while let Some(group) = pending.pop() {
        let same = apply(fragment, start, &group).is_some_and(|corrected| {
            let corrected = (equivalence.preprocess)(&corrected);
            let result = crate::parser::parse_as(&corrected, version);
            result.errors().next().is_none()
                && result.depth_error().is_none()
                && (equivalence.fingerprint)(&result.node(), &corrected) == *expected
        });
        if same {
            accepted.extend(group);
        } else if group.len() > 1 {
            let half = group.len() / 2;
            pending.push(group[half..].to_vec());
            pending.push(group[..half].to_vec());
        }
    }
    accepted
}

/// The byte ranges of every `def`, `class`, `module` and `class << x`,
/// sorted by start (RuboCop's reparse scopes).
fn reparse_scopes(program: &Node<'_>) -> Vec<Range<usize>> {
    struct Scopes(Vec<Range<usize>>);
    impl<'pr> NodeVisitor<'pr> for Scopes {
        fn enter(&mut self, node: Node<'pr>) {
            if matches!(
                node,
                Node::DefNode { .. }
                    | Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::SingletonClassNode { .. }
            ) {
                let location = node.location();
                self.0.push(location.start_offset()..location.end_offset());
            }
        }
    }
    let mut scopes = Scopes(Vec::new());
    walk(&mut scopes, program);
    scopes
        .0
        .sort_by_key(|scope| (scope.start, std::cmp::Reverse(scope.end)));
    scopes.0
}

/// No preprocessing.
pub(crate) fn unchanged(source: &[u8]) -> Cow<'_, [u8]> {
    Cow::Borrowed(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equivalence() -> Equivalence {
        Equivalence {
            fingerprint: super::super::ast::fingerprint_ignoring_grouping,
            preprocess: unchanged,
        }
    }

    /// Candidates replacing byte ranges of `source` with texts.
    fn check(source: &str, edits: &[(Range<usize>, &str)]) -> Vec<usize> {
        let file = SourceFile::new("t.rb".into(), source.as_bytes().to_vec());
        let parsed = crate::parser::parse(source.as_bytes());
        let spans: Vec<_> = edits.iter().map(|(r, _)| r.clone()).collect();
        let apply = |fragment: &[u8], start: usize, group: &[usize]| {
            let mut out = fragment.to_vec();
            let mut sorted = group.to_vec();
            sorted.sort_by_key(|&i| std::cmp::Reverse(edits[i].0.start));
            for i in sorted {
                let (range, text) = &edits[i];
                out.splice(range.start - start..range.end - start, text.bytes());
            }
            Some(out)
        };
        verified(&file, &parsed.node(), &spans, &apply, &equivalence())
    }

    #[test]
    fn groups_by_innermost_scope_and_bisects_failing_batches() {
        // `(1)` -> `1` keeps the tree; `(-2) ** i` -> `-2 ** i` does not.
        let source = "class A\n  def f(i)\n    x = (-2) ** i\n    y = (1)\n  end\nend\nz = (3)\n";
        let p = |s: &str| source.find(s).unwrap();
        let edits = [
            (p("(-2)")..p("(-2)") + 4, "-2"),
            (p("(1)")..p("(1)") + 3, "1"),
            (p("(3)")..p("(3)") + 3, "3"),
        ];
        assert_eq!(check(source, &edits), [1, 2]);
    }

    #[test]
    fn a_prepared_verifier_answers_each_call_like_a_fresh_verification() {
        let source = "class A\n  def f(i)\n    x = (-2) ** i\n    y = (1)\n  end\nend\nz = (3)\nw = (-4) ** 2\n";
        let p = |s: &str| source.find(s).unwrap();
        let edits = [
            (p("(-2)")..p("(-2)") + 4, "-2"),
            (p("(1)")..p("(1)") + 3, "1"),
            (p("(3)")..p("(3)") + 3, "3"),
            (p("(-4)")..p("(-4)") + 4, "-4"),
        ];
        let file = SourceFile::new("t.rb".into(), source.as_bytes().to_vec());
        let parsed = crate::parser::parse(source.as_bytes());
        let equivalence = equivalence();
        let mut verifier = Verifier::new(&parsed.node(), &equivalence);
        let answers: Vec<bool> = edits
            .iter()
            .map(|(range, text)| {
                let apply = |fragment: &[u8], start: usize, _: &[usize]| {
                    let mut out = fragment.to_vec();
                    out.splice(range.start - start..range.end - start, text.bytes());
                    Some(out)
                };
                !verifier
                    .verified(&file, &parsed.node(), std::slice::from_ref(range), &apply)
                    .is_empty()
            })
            .collect();
        assert_eq!(answers, [false, true, true, false]);
        assert_eq!(check(source, &edits), [1, 2]);
    }

    #[test]
    fn a_fragment_that_does_not_parse_alone_verifies_nothing() {
        let source = "def f = (<<~A)\n  a\nA\n";
        let p = source.find("(<<~A)").unwrap();
        assert_eq!(check(source, &[(p..p + 6, "<<~A")]), Vec::<usize>::new());
    }

    #[test]
    fn preprocessing_applies_to_both_sides() {
        fn neutral(source: &[u8]) -> Cow<'_, [u8]> {
            Cow::Owned(
                String::from_utf8_lossy(source)
                    .replace("__LINE__", "__LINE0__")
                    .into_bytes(),
            )
        }
        let source = "x = [__LINE__,\n  __LINE__]\n";
        let file = SourceFile::new("t.rb".into(), source.as_bytes().to_vec());
        let parsed = crate::parser::parse(source.as_bytes());
        let apply = |fragment: &[u8], _: usize, _: &[usize]| {
            Some(
                String::from_utf8_lossy(fragment)
                    .replace(",\n  ", ", ")
                    .into_bytes(),
            )
        };
        let strict = Equivalence {
            fingerprint: super::super::ast::fingerprint_ignoring_grouping,
            preprocess: unchanged,
        };
        let whole = 0..source.len();
        let spans = std::slice::from_ref(&whole);
        assert!(verified(&file, &parsed.node(), spans, &apply, &strict).is_empty());
        let lenient = Equivalence {
            fingerprint: super::super::ast::fingerprint_ignoring_grouping,
            preprocess: neutral,
        };
        assert_eq!(
            verified(&file, &parsed.node(), spans, &apply, &lenient),
            [0]
        );
    }
}
