//! Main-parse physical heredoc ownership shared by all correction entries.
//!
//! A successful reparse cannot detect a body line promoted to a terminator.
//! On each affected line we require an immutable byte witness: even deleting
//! the UNION of proposed changes must leave bytes that cannot occur in the
//! label. This overapproximates conflict, suppression, selection and alias
//! subsets, including adjacent edits. Work inspects bounded correction envelopes
//! and affected physical lines, never scans an enclosing whole body per edit.
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};

use super::SourceFile;
use crate::offense::{Correction, Offense};

/// Local proof budget, not a syntax/detection limit. Larger affected envelopes,
/// lines or labels retain diagnostics without edits. Main-AST indexing costs
/// O(nodes * depth); batch work is bounded local bytes plus O(edits log edits).
const PROOF_BYTES: usize = 4096;
/// The empty string literal that replaces an empty heredoc, in either quotes.
const EMPTY_STRINGS: [&[u8]; 2] = [b"\"\"", b"''"];

#[derive(Clone, Debug)]
pub(crate) struct Heredoc {
    pub opening: Range<usize>,
    pub body: Range<usize>,
    pub closing: Range<usize>,
    pub label: Range<usize>,
    pub indented: bool,
}
#[derive(Debug, Default)]
pub(crate) struct Boundaries {
    heredocs: Vec<Heredoc>,
    line_starts: Arc<Vec<usize>>,
    anchors: Vec<Range<usize>>,
    tokens: Vec<Range<usize>>,
    openings: Vec<Range<usize>>,
    closing_starts: Vec<usize>,
    /// `(closing line start, owner)`, sorted by closing line start.
    closings: Vec<(usize, usize)>,
    segments: Vec<Segment>,
    equivalent_quotes: BTreeSet<usize>,
}
#[derive(Debug)]
struct Segment {
    range: Range<usize>,
    owners: Vec<usize>,
}
struct Change<'a> {
    range: Range<usize>,
    replacement: &'a [u8],
    // Only an exact, main-parse-proved structural operation can name an owner.
    boundary: Option<usize>,
    /// Exact atomic uppercase rename; its entire envelope excludes peer writes.
    rename: bool,
    /// Canonical heredoc indentation edit, proved by `reindentation`.
    reindent: bool,
}

impl Boundaries {
    pub(super) fn build(file: &SourceFile, heredocs: &[Heredoc]) -> Self {
        let mut sorted = heredocs.to_vec();
        sorted.sort_unstable_by_key(|h| h.opening.start);
        let heredocs = sorted.as_slice();
        let mut tokens = Vec::new();
        let mut openings = Vec::new();
        let mut closing_starts = Vec::new();
        let mut anchors = Vec::new();
        let mut events = Vec::new();
        for (id, h) in heredocs.iter().enumerate() {
            openings.push(h.opening.clone());
            closing_starts.push(h.closing.start);
            anchors.push(h.opening.clone());
            let closing_line = file.line_of(h.closing.start);
            let closing_text_end = h.closing.start + file.line_text(closing_line).len();
            if h.closing.start < closing_text_end {
                anchors.push(h.closing.start..closing_text_end);
            }
            let line = file.line_of(h.opening.end);
            tokens.push(
                file.line_start(line) + file.line_text(line).len()..file.line_start(line + 1),
            );
            tokens.push(h.closing.clone());
            if !h.body.is_empty() {
                events.push((h.body.start, true, id));
                events.push((h.body.end, false, id));
            }
        }
        tokens.sort_unstable_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in tokens {
            if range.is_empty() {
                continue;
            }
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        openings.sort_unstable_by_key(|r| r.start);
        closing_starts.sort_unstable();
        anchors.sort_unstable_by_key(|r| r.start);
        events.sort_unstable();
        let mut active = BTreeSet::new();
        let mut previous = 0;
        let mut segments = Vec::new();
        for (offset, enter, id) in events {
            if previous < offset && !active.is_empty() {
                segments.push(Segment {
                    range: previous..offset,
                    owners: active.iter().copied().collect(),
                });
            }
            if enter {
                active.insert(id);
            } else {
                active.remove(&id);
            }
            previous = offset;
        }
        // Single-quoted bodies cannot contain nested syntax. Inspect their
        // character heads once per main owner, never per proposed edit.
        let equivalent_quotes = heredocs
            .iter()
            .enumerate()
            .filter_map(|(id, h)| {
                let raw = &file.bytes[h.opening.clone()];
                let prefix = 2 + usize::from(h.indented);
                let quote = *raw.get(prefix)?;
                let label = &file.bytes[h.label.clone()];
                if !matches!(quote, b'\'' | b'"')
                    || !label
                        .first()
                        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                    || !label
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    return None;
                }
                if quote == b'\''
                    && file.character_offsets(h.body.clone()).any(|i| {
                        file.bytes[i] == b'\\'
                            || file.bytes[i] == b'#'
                                && matches!(file.bytes.get(i + 1), Some(b'{' | b'@' | b'$'))
                    })
                {
                    return None;
                }
                Some(id)
            })
            .collect();
        let mut closings: Vec<_> = heredocs
            .iter()
            .enumerate()
            .map(|(id, h)| (h.closing.start, id))
            .collect();
        closings.sort_unstable();
        Self {
            equivalent_quotes,
            closings,
            heredocs: heredocs.to_vec(),
            line_starts: file.line_starts.clone(),
            anchors,
            tokens: merged,
            openings,
            closing_starts,
            segments,
        }
    }

    /// Bounded exact main-owner rename, with no quote/body/interpolation changes.
    /// Encoded/non-ASCII labels and opaque String envelopes retain diagnostics.
    pub(crate) fn uppercase_correction(&self, source: &[u8], opening: usize) -> Option<Correction> {
        let id = self.heredocs.partition_point(|h| h.opening.start < opening);
        let h = self
            .heredocs
            .get(id)
            .filter(|h| h.opening.start == opening)?;
        let label = &source[h.label.clone()];
        if h.closing.end - opening > PROOF_BYTES
            || !label.is_ascii()
            || !label.iter().any(u8::is_ascii_lowercase)
            || label
                .iter()
                .any(|b| matches!(b, b'\'' | b'"' | b'`' | b'\r' | b'\n'))
        {
            return None;
        }
        let upper = label.to_ascii_uppercase();
        if source[h.body.clone()]
            .split_inclusive(|&b| b == b'\n')
            .any(|line| {
                let line = line.strip_suffix(b"\n").unwrap_or(line);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                // Unlike the detector's String#strip predicate, preserve the
                // label's own whitespace. A quoted ` sql ` can terminate on
                // ` SQL ` even though strip no longer equals the full label.
                line.strip_suffix(upper.as_slice()).is_some_and(|prefix| {
                    prefix.is_empty() || h.indented && prefix.iter().copied().all(inline_whitespace)
                })
            })
        {
            return None;
        }
        let closing = &source[h.closing.clone()];
        let eol = usize::from(closing.ends_with(b"\n")) + usize::from(closing.ends_with(b"\r\n"));
        let end = h.closing.end - eol;
        let start = end.checked_sub(label.len())?;
        if source.get(start..end)? != label
            || !source[h.closing.start..start]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
        {
            return None;
        }
        let mut replacement = source[opening..h.closing.end].to_vec();
        replacement[h.label.start - opening..h.label.end - opening].copy_from_slice(&upper);
        replacement[start - opening..end - opening].copy_from_slice(&upper);
        Some(Correction {
            range: opening..h.closing.end,
            replacement: String::from_utf8(replacement).ok()?,
        })
    }

    fn line(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }
    fn line_text<'a>(&self, source: &'a [u8], line: usize) -> &'a [u8] {
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(source.len());
        let text = &source[self.line_starts[line]..end];
        let text = text.strip_suffix(b"\n").unwrap_or(text);
        text.strip_suffix(b"\r").unwrap_or(text)
    }

    /// The two canonical heredoc indentation edits, proved from the
    /// main-parse owner and the bounded edit bytes alone:
    ///
    /// - an opener `<<`/`<<-` becoming `<<~`. Already indented owners keep
    ///   their terminator set; a plain `<<` owner qualifies only when none of
    ///   its body lines could spell its label after optional indentation.
    /// - a whitespace-prefix rewrite of physical lines from an indented
    ///   owner's body start (or its closing line) through the closing line's
    ///   indentation. Every line keeps its bytes after the prefix, the owner and
    ///   every heredoc opened inside the envelope are indented (their
    ///   terminators ignore indentation), and those lines belong to the
    ///   owner's lexing, never to an enclosing body.
    ///
    /// Literal values are the emitting cop's proof; this proves terminators.
    fn reindentation(&self, source: &[u8], edit: &Correction) -> Option<usize> {
        let r = &edit.range;
        let new = edit.replacement.as_bytes();
        let id = self.heredocs.partition_point(|h| h.opening.start < r.start);
        if let Some(h) = self.heredocs.get(id).filter(|h| h.opening.start == r.start) {
            let prefix = 2 + usize::from(h.indented);
            let squiggly = source.get(h.opening.start + 2) == Some(&b'~');
            let label = &source[h.label.clone()];
            let proved = !squiggly
                && r.end == r.start + prefix
                && new == b"<<~"
                && (h.indented
                    || h.closing.end - h.body.start <= PROOF_BYTES
                        && !source[h.body.clone()].split(|&b| b == b'\n').any(|line| {
                            // A blank line spells no label with a
                            // non-blank byte; the subsequence test
                            // overapproximates every other line.
                            (line.iter().any(|&b| !inline_whitespace(b))
                                || label.iter().all(|&b| inline_whitespace(b)))
                                && could_be_label(line, label, true)
                        }));
            return proved.then_some(id);
        }
        let at = self.closings.partition_point(|&(start, _)| start <= r.end);
        let &(closing_start, id) = self.closings.get(at.checked_sub(1)?)?;
        let h = &self.heredocs[id];
        let closing_text = &source[closing_start..h.closing.end];
        let indentation = closing_text
            .iter()
            .take_while(|&&b| matches!(b, b' ' | b'\t'))
            .count();
        if !h.indented
            || r.end > closing_start + indentation
            || r.start != h.body.start && r.start != closing_start
        {
            return None;
        }
        let old = &source[r.clone()];
        if memchr::memchr_iter(b'\n', old).count() != memchr::memchr_iter(b'\n', new).count()
            || self.heredocs[id + 1..]
                .iter()
                .take_while(|nested| nested.opening.start < r.end)
                .any(|nested| !nested.indented)
        {
            return None;
        }
        let same_text = |old: &[u8], new: &[u8]| {
            let old_prefix = old
                .iter()
                .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'))
                .count();
            let new_prefix = new.iter().take_while(|&&b| b == b' ').count();
            old[old_prefix..] == new[new_prefix..]
        };
        let lines = old.split(|&b| b == b'\n').zip(new.split(|&b| b == b'\n'));
        lines
            .into_iter()
            .all(|(o, n)| same_text(o, n))
            .then_some(id)
    }

    /// Atomic cops sometimes retain openings/closings inside their replacement
    /// envelopes. Match ONLY unique main-parse boundary anchors, then equal-count
    /// physical lines, to expose actual changes. This is a bounded byte proof,
    /// not candidate parsing, a lexer, or a general-purpose diff algorithm.
    fn changes<'a>(&self, source: &[u8], edit: &'a Correction) -> Option<Vec<Change<'a>>> {
        if edit.range.len() > PROOF_BYTES || edit.replacement.len() > PROOF_BYTES {
            return None;
        }
        let replacement = edit.replacement.as_bytes();
        if let Some(owner) = self.reindentation(source, edit) {
            return Some(vec![Change {
                range: edit.range.clone(),
                replacement,
                boundary: Some(owner),
                rename: false,
                reindent: true,
            }]);
        }
        let first = self
            .heredocs
            .partition_point(|h| h.opening.start < edit.range.start);
        // No candidate parse or whole-body edit-time scan.
        if let Some(h) = self.heredocs.get(first)
            && h.opening.start == edit.range.start
        {
            if let Some(canonical) = self.uppercase_correction(source, h.opening.start)
                && canonical.range == edit.range
                && canonical.replacement == edit.replacement
            {
                let closing = h.closing.start - h.opening.start;
                return Some(vec![
                    Change {
                        range: h.opening.clone(),
                        replacement: &replacement[..h.opening.len()],
                        boundary: Some(first),
                        rename: true,
                        reindent: false,
                    },
                    Change {
                        range: h.closing.clone(),
                        replacement: &replacement[closing..],
                        boundary: Some(first),
                        rename: true,
                        reindent: false,
                    },
                ]);
            }
            let prefix = 2 + usize::from(h.indented);
            let raw = &source[h.opening.clone()];
            let quote_edit = edit.range == h.opening
                && self.equivalent_quotes.contains(&first)
                && replacement == [&raw[..prefix], &source[h.label.clone()]].concat();
            if quote_edit {
                return Some(vec![Change {
                    range: h.opening.clone(),
                    replacement,
                    boundary: Some(first),
                    rename: false,
                    reindent: false,
                }]);
            }
            let empty_edit = h.body.is_empty()
                && raw.get(prefix) != Some(&b'`')
                && edit.range.end == h.closing.end
                && EMPTY_STRINGS
                    .iter()
                    .find_map(|empty| replacement.strip_prefix(*empty))
                    == Some(&source[h.opening.end..h.closing.start])
                && (h.closing.end == source.len()
                    || self.line_starts.binary_search(&h.closing.end).is_ok());
            if empty_edit {
                return Some(vec![
                    Change {
                        range: h.opening.clone(),
                        replacement: &replacement[..2],
                        boundary: Some(first),
                        rename: false,
                        reindent: false,
                    },
                    Change {
                        range: h.closing.clone(),
                        replacement: b"",
                        boundary: Some(first),
                        rename: false,
                        reindent: false,
                    },
                ]);
            }
        }
        self.anchored(source, edit)
            .or_else(|| kept_middle(source, edit))
    }

    /// Decompose an envelope around its unique main-parse anchors.
    fn anchored<'a>(&self, source: &[u8], edit: &'a Correction) -> Option<Vec<Change<'a>>> {
        let replacement = edit.replacement.as_bytes();
        let mut old = edit.range.start;
        let mut new = 0;
        let mut changes = Vec::new();
        let first = self.anchors.partition_point(|a| a.end <= old);
        for anchor in self.anchors[first..]
            .iter()
            .take_while(|a| a.start < edit.range.end)
        {
            if anchor.start < old || anchor.end > edit.range.end {
                continue;
            }
            let bytes = &source[anchor.clone()];
            let tail = &replacement[new..];
            let at = memchr::memmem::find(tail, bytes)?;
            // Repeated spellings cannot establish which physical owner survived.
            if memchr::memmem::find(&tail[at + 1..], bytes).is_some() {
                return None;
            }
            line_changes(source, old..anchor.start, &tail[..at], &mut changes);
            old = anchor.end;
            new += at + bytes.len();
        }
        line_changes(
            source,
            old..edit.range.end,
            &replacement[new..],
            &mut changes,
        );
        Some(changes)
    }

    fn relevant(&self, r: &Range<usize>) -> bool {
        let token = self.tokens.partition_point(|t| t.end <= r.start);
        let opening = self.openings.partition_point(|t| t.end < r.start);
        let body = self.segments.partition_point(|s| s.range.end <= r.start);
        let pending = self
            .openings
            .partition_point(|t| t.start < self.line_starts[self.line(r.start)]);
        self.tokens.get(token).is_some_and(|t| touches(r, t))
            || self.openings.get(opening).is_some_and(|t| t.start <= r.end)
            || self
                .segments
                .get(body)
                .is_some_and(|s| touches(r, &s.range))
            || self
                .openings
                .get(pending)
                .is_some_and(|t| t.start < r.end || r.is_empty() && t.start < r.start)
    }

    /// Subset-safe, not a simulation of today's conflict winners. The pipeline
    /// repeats this over the alias union with the SAME main-parse/line evidence.
    pub(crate) fn filter(&self, source: &[u8], offenses: &mut [Offense]) {
        if self.heredocs.is_empty() {
            return;
        }
        let mut lines: BTreeMap<usize, Vec<(usize, Range<usize>, bool)>> = BTreeMap::new();
        let mut rejected = BTreeSet::new();
        // Decompose each bounded envelope once. Read dependencies (closing EOL
        // transfers and leading/trailing opener separators) must survive every
        // subset, including writes by adjacent edits in a different envelope.
        // Use the byte-exact changes, not entire atomic envelopes: unchanged
        // bytes within an envelope can still prove a separator.
        let changes: Vec<_> = offenses
            .iter()
            .map(|o| {
                let edit = o.correction.as_ref()?;
                let r = &edit.range;
                if r.start > r.end || r.end > source.len() {
                    return None;
                }
                if !self.relevant(r) {
                    // Unrelated edits need no envelope scan, but their raw
                    // writes can invalidate a boundary's read dependency.
                    return Some(vec![Change {
                        range: r.clone(),
                        replacement: edit.replacement.as_bytes(),
                        boundary: None,
                        rename: false,
                        reindent: false,
                    }]);
                }
                self.changes(source, edit)
            })
            .collect();
        let mut writes: Vec<Range<usize>> = changes
            .iter()
            .flatten()
            .flatten()
            .map(|c| c.range.clone())
            .filter(|r| !r.is_empty())
            .collect();
        writes.sort_unstable_by_key(|r| (r.start, r.end));
        let mut merged_writes: Vec<Range<usize>> = Vec::new();
        for r in writes {
            if let Some(last) = merged_writes.last_mut()
                && r.start <= last.end
            {
                last.end = last.end.max(r.end);
            } else {
                merged_writes.push(r);
            }
        }
        let points: BTreeSet<usize> = changes
            .iter()
            .flatten()
            .flatten()
            .filter(|c| c.range.is_empty())
            .map(|c| c.range.start)
            .collect();
        let untouched = |dependency: Range<usize>| {
            let first = merged_writes.partition_point(|w| w.end <= dependency.start);
            !merged_writes
                .get(first)
                .is_some_and(|w| touches(&dependency, w))
                && points.range(dependency).next().is_none()
        };
        for (id, offense) in offenses.iter().enumerate() {
            let Some(edit) = &offense.correction else {
                continue;
            };
            let r = &edit.range;
            // Invalid ranges remain the correction engine's diagnostic.
            if r.start > r.end || r.end > source.len() {
                continue;
            }
            if !self.relevant(r) {
                continue;
            }
            let Some(changes) = &changes[id] else {
                rejected.insert(id);
                continue;
            };
            for change in changes {
                let r = &change.range;
                if change.reindent {
                    let owner = change.boundary.expect("reindentation names its owner");
                    let h = &self.heredocs[owner];
                    if r.start != h.opening.start {
                        // Prefix rewrite: overlapping peers conflict; the lines
                        // are the owner's own lexing, proved in `changes`.
                        continue;
                    }
                    // A plain owner's new terminator set, and the emitting
                    // cop's zero-dedent value proof, hold for the original
                    // body lines: no peer may write them (closing-line
                    // indentation is neither). An enclosing owner sees the
                    // changed opener line.
                    let line = self.line(r.start);
                    let text = self.line_text(source, line);
                    let start = self.line_starts[line];
                    let end = start + text.len();
                    let first = self.segments.partition_point(|s| s.range.end <= r.start);
                    let enclosing: Vec<usize> = self
                        .segments
                        .get(first)
                        .filter(|s| s.range.start <= r.start)
                        .map(|s| s.owners.iter().copied().filter(|&o| o != owner).collect())
                        .unwrap_or_default();
                    let changed = [
                        &text[..r.start - start],
                        change.replacement,
                        &text[r.end - start..],
                    ]
                    .concat();
                    if !untouched(h.body.clone())
                        || !enclosing.is_empty()
                            && (text.len() > PROOF_BYTES
                                || !untouched(start..r.start)
                                || !untouched(r.end..end)
                                || enclosing.iter().any(|&o| {
                                    let e = &self.heredocs[o];
                                    could_be_label(&changed, &source[e.label.clone()], e.indented)
                                }))
                    {
                        rejected.insert(id);
                    }
                    continue;
                }
                if change.rename {
                    // Both anchors change atomically; every interior byte is
                    // immutable when this envelope wins. Only enclosing owners
                    // can observe these changed physical lines.
                    let line = self.line(r.start);
                    let text = self.line_text(source, line);
                    let start = self.line_starts[line];
                    // The opener's preceding code is outside this atomic
                    // envelope. Its witness must survive the union of peers.
                    if text.len() > PROOF_BYTES || start < r.start && !untouched(start..r.start) {
                        rejected.insert(id);
                        continue;
                    }
                    let mut changed = text.to_vec();
                    let end = r.end.min(start + text.len());
                    changed[r.start - start..end - start]
                        .copy_from_slice(&change.replacement[..end - r.start]);
                    let first = self.segments.partition_point(|s| s.range.end <= r.start);
                    for segment in self.segments[first..]
                        .iter()
                        .take_while(|s| s.range.start < r.end)
                    {
                        for &owner in &segment.owners {
                            if Some(owner) == change.boundary {
                                continue;
                            }
                            let h = &self.heredocs[owner];
                            if could_be_label(&changed, &source[h.label.clone()], h.indented) {
                                rejected.insert(id);
                            }
                        }
                    }
                    continue;
                }
                let structural = change.boundary.map(|id| &self.heredocs[id]);
                if let Some(h) = structural {
                    if *r == h.closing {
                        // A whole raw-empty owner's terminator line, not an
                        // arbitrary boundary or an enclosing owner's text.
                        continue;
                    }
                    // Quote removal exposes a suffix: require its separator to
                    // remain immutable in the union of all proposed writes.
                    if !EMPTY_STRINGS.contains(&change.replacement)
                        && source.get(r.end).is_some_and(|b| could_continue_name(*b))
                        || source.get(r.end).is_some() && !untouched(r.end..r.end + 1)
                    {
                        rejected.insert(id);
                        continue;
                    }
                }
                let token = self.tokens.partition_point(|t| t.end <= r.start);
                let opening = self.openings.partition_point(|t| t.end < r.start);
                let line = self.line(r.start);
                let line_start = self.line_starts[line];
                let prior = self.openings.partition_point(|t| t.start < line_start);
                let newline = change
                    .replacement
                    .iter()
                    .any(|b| matches!(b, b'\r' | b'\n'));
                let eol_start = line_start + self.line_text(source, line).len();
                let eol_end = self
                    .line_starts
                    .get(line + 1)
                    .copied()
                    .unwrap_or(source.len());
                let eol = &source[eol_start..eol_end];
                let at_closing_eol = r.start == eol_start
                    && !eol.is_empty()
                    && self.closing_starts.binary_search(&line_start).is_ok();
                if at_closing_eol {
                    if r.is_empty() && change.replacement.starts_with(eol) {
                        continue;
                    }
                    let dependency = r.end..r.end.saturating_add(eol.len());
                    if change.replacement.is_empty()
                        && source[r.clone()].starts_with(eol)
                        && memchr::memchr_iter(b'\n', &source[r.clone()]).count() == 1
                        && source.get(dependency.clone()) == Some(eol)
                        && untouched(dependency)
                    {
                        continue;
                    }
                }
                // Whitespace before `<<` is significant in command arguments:
                // deleting it can turn a parsed heredoc into a shift expression.
                // A deletion ending at the opening must expose immutable
                // whitespace or an expression-opening `(`/`[`. Check the union, not
                // just this edit, including byte-exact atomic-envelope changes.
                // Insertions/replacements ending here must themselves retain
                // proved separators; other prefixes have uncertain ownership.
                let next = self.openings.partition_point(|t| t.start < r.end);
                let removes_leading_separator = self.openings.get(next).is_some_and(|t| {
                    t.start == r.end
                        && if change.replacement.is_empty() {
                            r.start == 0
                                || !opening_separator(source[r.start - 1])
                                || !untouched(r.start - 1..r.start)
                        } else {
                            !change.replacement.iter().copied().all(opening_separator)
                        }
                });
                let modifies_opening = self.openings.get(opening).is_some_and(|t| {
                    if r.is_empty() && r.start == t.start {
                        return false; // Leading-separator proof above.
                    }
                    if r.start == t.end {
                        let first = change
                            .replacement
                            .first()
                            .copied()
                            .or_else(|| source.get(r.end).copied());
                        return first.is_some_and(could_continue_name)
                            || change.replacement.is_empty()
                                && source.get(r.end).is_some()
                                && !untouched(r.end..r.end + 1);
                    }
                    touches(r, t)
                });
                if removes_leading_separator
                    || modifies_opening && structural.is_none()
                    || self.tokens.get(token).is_some_and(|t| {
                        touches(r, t)
                            && !(r.is_empty()
                                && r.start == t.start
                                && !newline
                                && matches!(source.get(t.start), Some(b'\r' | b'\n')))
                    })
                    || newline && self.openings.get(prior).is_some_and(|t| t.start < r.start)
                {
                    rejected.insert(id);
                    continue;
                }
                let body = self.segments.partition_point(|s| s.range.end <= r.start);
                if !self
                    .segments
                    .get(body)
                    .is_some_and(|s| touches(r, &s.range))
                {
                    continue;
                }
                // An atomic structural replacement supplies its own immutable
                // witness when selected. Overlapping peer writes cannot also be
                // applied; every enclosing owner must exclude this exact token.
                if structural.is_some()
                    && self.segments[body].owners.iter().all(|&owner| {
                        let h = &self.heredocs[owner];
                        !could_be_label(change.replacement, &source[h.label.clone()], h.indented)
                    })
                {
                    continue;
                }
                // Removing complete blank code lines cannot join or manufacture
                // a terminator line; existing literal protection proves values.
                if change.replacement.is_empty()
                    && r.start == line_start
                    && self.line_starts.binary_search(&r.end).is_ok()
                    && source[r.clone()].iter().all(u8::is_ascii_whitespace)
                {
                    continue;
                }
                let end = line_start + self.line_text(source, line).len();
                if r.end > end
                    || newline && memchr::memchr_iter(b'\n', change.replacement).count() != 1
                {
                    rejected.insert(id);
                } else {
                    lines
                        .entry(line)
                        .or_default()
                        .push((id, r.clone(), newline));
                }
            }
        }
        for (line, mut edits) in lines {
            let start = self.line_starts[line];
            let text = self.line_text(source, line);
            let segment = &self.segments[self.segments.partition_point(|s| s.range.end <= start)];
            edits.sort_unstable_by_key(|(_, r, _)| (r.start, r.end));
            // A possible inserted newline splits the mandatory evidence. Every
            // piece must independently exclude the label; joining any subset
            // of pieces therefore excludes it too. Replacement bytes are not
            // needed as evidence, and conflicting split points overapproximate.
            let splits: BTreeSet<_> = edits
                .iter()
                .filter(|(_, _, split)| *split)
                .map(|(_, r, _)| r.start)
                .collect();
            let mut mandatory = vec![Vec::new()];
            if text.len() <= PROOF_BYTES {
                let mut next = 0;
                let mut removed_end = start;
                for (local, byte) in text
                    .iter()
                    .map(Some)
                    .chain(std::iter::once(None))
                    .enumerate()
                {
                    let offset = start + local;
                    if splits.contains(&offset) {
                        mandatory.push(Vec::new());
                    }
                    while let Some((_, range, _)) = edits.get(next)
                        && range.start <= offset
                    {
                        removed_end = removed_end.max(range.end);
                        next += 1;
                    }
                    if offset >= removed_end
                        && let Some(&byte) = byte
                    {
                        mandatory.last_mut().unwrap().push(byte);
                    }
                }
            }
            let proved = text.len() <= PROOF_BYTES
                && segment.owners.iter().all(|&owner| {
                    let h = &self.heredocs[owner];
                    let label = &source[h.label.clone()];
                    label.len() <= PROOF_BYTES
                        && mandatory
                            .iter()
                            .all(|piece| !could_be_label(piece, label, h.indented))
                });
            if !proved {
                rejected.extend(edits.iter().map(|(id, _, _)| *id));
            }
        }
        for id in rejected {
            offenses[id].correction = None;
        }
    }
}

/// Envelope edges a kept-middle decomposition may rewrite.
const EDGE_BYTES: usize = 64;

/// An exact decomposition of an envelope whose replacement keeps one middle
/// run of original bytes: a bounded rewrite of its start and a bounded
/// deletion at its end (a closing parenthesis moved before heredoc bodies).
/// Each part then meets the ordinary per-change checks; no bytes are guessed.
fn kept_middle<'a>(source: &[u8], edit: &'a Correction) -> Option<Vec<Change<'a>>> {
    let old = &source[edit.range.clone()];
    let new = edit.replacement.as_bytes();
    for inserted in 0..=new.len().min(EDGE_BYTES) {
        let middle = &new[inserted..];
        let Some(spare) = old.len().checked_sub(middle.len()) else {
            continue;
        };
        for removed in 0..=spare.min(EDGE_BYTES) {
            if spare - removed > EDGE_BYTES || old[removed..removed + middle.len()] != *middle {
                continue;
            }
            let start = edit.range.start;
            let tail = start + removed + middle.len();
            let changes = [
                Change {
                    range: start..start + removed,
                    replacement: &new[..inserted],
                    boundary: None,
                    rename: false,
                    reindent: false,
                },
                Change {
                    range: tail..edit.range.end,
                    replacement: b"",
                    boundary: None,
                    rename: false,
                    reindent: false,
                },
            ];
            return Some(
                changes
                    .into_iter()
                    .filter(|c| !(c.range.is_empty() && c.replacement.is_empty()))
                    .collect(),
            );
        }
    }
    None
}

fn line_changes<'a>(
    source: &[u8],
    range: Range<usize>,
    replacement: &'a [u8],
    out: &mut Vec<Change<'a>>,
) {
    let before = &source[range.clone()];
    let count = |bytes: &[u8]| memchr::memchr_iter(b'\n', bytes).count();
    if count(before) == count(replacement) {
        let mut offset = range.start;
        for (old, new) in before
            .split_inclusive(|&b| b == b'\n')
            .zip(replacement.split_inclusive(|&b| b == b'\n'))
        {
            trimmed_change(offset, old, new, out);
            offset += old.len();
        }
        // split_inclusive yields no final empty fragment: preserve insertions
        // into an empty range, and text after a final unchanged newline.
        let old_tail = before.rsplit(|&b| b == b'\n').next().unwrap();
        let new_tail = replacement.rsplit(|&b| b == b'\n').next().unwrap();
        if old_tail.is_empty() != new_tail.is_empty() {
            trimmed_change(range.end - old_tail.len(), old_tail, new_tail, out);
        }
    } else {
        trimmed_change(range.start, before, replacement, out);
    }
}
fn trimmed_change<'a>(offset: usize, old: &[u8], new: &'a [u8], out: &mut Vec<Change<'a>>) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if prefix + suffix < old.len() || prefix + suffix < new.len() {
        out.push(Change {
            range: offset + prefix..offset + old.len() - suffix,
            replacement: &new[prefix..new.len() - suffix],
            boundary: None,
            rename: false,
            reindent: false,
        });
    }
}
fn touches(edit: &Range<usize>, region: &Range<usize>) -> bool {
    edit.start < region.end
        && (edit.end > region.start || edit.is_empty() && edit.start >= region.start)
}
/// Prism's unquoted heredoc labels use char_is_identifier (not Unicode
/// alphabetic properties): ASCII alnum/underscore, or any valid character of
/// the declared encoding at a high byte. Without candidate encoding facts,
/// every high byte is uncertain, including invalid/truncated UTF-8 or legacy
/// sequences. Never treat one as a proved delimiter-name separator.
fn could_continue_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
}

// Parentheses/brackets put Prism in expression-begin state, so canonical
// argument/array wrappers need no whitespace before their heredoc argument.
fn opening_separator(byte: u8) -> bool {
    inline_whitespace(byte) || matches!(byte, b'(' | b'[')
}

fn inline_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

fn could_be_label(mandatory: &[u8], label: &[u8], indented: bool) -> bool {
    let mut remaining = label;
    for &byte in mandatory {
        // Ignore ALL Prism inline whitespace (pm_char_is_inline_whitespace),
        // conservatively approximating optional leading heredoc indentation.
        // VT, FF and CR can precede a terminator too, so none is a witness.
        if indented && inline_whitespace(byte) {
            continue;
        }
        let Some(index) = remaining.iter().position(|&b| b == byte) else {
            return false;
        };
        remaining = &remaining[index + 1..];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cops::{Cop, NodeContext, run_cops, run_cops_with_corrections},
        offense::Severity,
    };
    use ruby_prism::Node;

    struct Probe {
        stage: u8,
        edits: Vec<Correction>,
    }
    impl Probe {
        fn emit(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            for edit in &self.edits {
                let (line, col) = file.line_col(edit.range.start);
                out.push(Offense {
                    cop: self.name(),
                    severity: Severity::Convention,
                    line,
                    col,
                    end_line: line,
                    message: "boundary probe".into(),
                    correction: Some(edit.clone()),
                });
            }
        }
    }
    impl Cop for Probe {
        fn name(&self) -> &'static str {
            "Style/BoundaryProbe"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            if self.stage == 0 {
                self.emit(file, out);
            }
        }
        fn check_node<'pr>(
            &self,
            node: &Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if self.stage == 1 && node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
        fn wants_leave_node(&self) -> bool {
            self.stage == 2
        }
        fn leave_node<'pr>(
            &self,
            node: &Node<'pr>,
            _: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                self.emit(file, out);
            }
        }
    }
    fn edit(range: Range<usize>, replacement: &str) -> Correction {
        Correction {
            range,
            replacement: replacement.into(),
        }
    }
    fn run(source: &str, edits: Vec<Correction>, stage: u8) -> Vec<Offense> {
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0, "{source}");
        let mut offenses = Vec::new();
        assert!(run_cops(&[&Probe { stage, edits }], &parsed, &file, &mut offenses).is_empty());
        offenses
    }

    #[test]
    fn ns06_atomic_rename_requires_exact_tokens_bounded_lines_and_immutable_prefixes() {
        let source = "p <<~sql\nvalue\nsql\n";
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        let index = file.heredoc_boundaries(&parsed.node());
        let rename = index.uppercase_correction(&file.bytes, 2).unwrap();
        for stage in 0..3 {
            assert!(
                run(source, vec![rename.clone()], stage)[0]
                    .correction
                    .is_some()
            );
            // A nonoverlapping peer can alter the opener line's read witness.
            assert!(
                run(source, vec![rename.clone(), edit(0..1, "")], stage)[0]
                    .correction
                    .is_none()
            );
            let partial = edit(2..8, "<<~SQL");
            assert!(run(source, vec![partial], stage)[0].correction.is_none());
            let mut corrupt = rename.clone();
            corrupt.replacement = corrupt.replacement.replace("value", "other");
            assert!(run(source, vec![corrupt], stage)[0].correction.is_none());
        }
        let long = format!("p '{}'; p <<~sql\nvalue\nsql\n", "x".repeat(PROOF_BYTES));
        let start = long.find("<<").unwrap();
        let file = SourceFile::new("test.rb".into(), long.as_bytes().to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        let rename = file
            .heredoc_boundaries(&parsed.node())
            .uppercase_correction(&file.bytes, start)
            .unwrap();
        assert!(run(&long, vec![rename], 0)[0].correction.is_none());
    }

    #[test]
    fn canonical_heredoc_reindentation_is_structural() {
        let accepted = |source: &str, edits: Vec<Correction>| {
            (0..3)
                .map(|stage| run(source, edits.clone(), stage)[0].correction.is_some())
                .collect::<Vec<_>>()
        };
        // Opener: `<<`/`<<-` to `<<~`.
        assert_eq!(
            accepted("p <<A\nfoo\nA\n", vec![edit(2..4, "<<~")]),
            [true; 3]
        );
        assert_eq!(
            accepted("p <<-A\nfoo\nA\n", vec![edit(2..5, "<<~")]),
            [true; 3]
        );
        // A plain body line spelling the label after indentation would end it.
        assert_eq!(
            accepted("p <<A\nfoo\n  A\nA\n", vec![edit(2..4, "<<~")]),
            [false; 3]
        );
        // A peer writing the plain owner's body invalidates the proof.
        assert!(
            !accepted(
                "p <<A\nfoo\nA\n",
                vec![edit(2..4, "<<~"), edit(6..9, "  A")]
            )[0]
        );
        // Not the canonical opener edit.
        assert_eq!(
            accepted("p <<-A\nfoo\nA\n", vec![edit(2..5, "<<")]),
            [false; 3]
        );
        assert_eq!(
            accepted("p <<~A\nfoo\nA\n", vec![edit(2..5, "<<~")]),
            [true; 3]
        );
        // Body and closing prefixes of an indented owner.
        let source = "p <<~A\nfoo\n bar\nA\n";
        assert_eq!(
            accepted(source, vec![edit(7..16, "  foo\n   bar\n  ")]),
            [true; 3]
        );
        assert_eq!(accepted(source, vec![edit(16..16, "  ")]), [true; 3]);
        // Text after the prefix must not change; a plain owner never qualifies.
        assert_eq!(
            accepted(source, vec![edit(7..16, "  fox\n   bar\n  ")]),
            [false; 3]
        );
        assert_eq!(
            accepted("p <<A\nfoo\nA\n", vec![edit(10..10, "  ")]),
            [false; 3]
        );
        // A plain heredoc opened inside the envelope would lose its terminator.
        let nested = "p <<~A\n#{<<B}\nb\nB\nA\n";
        assert_eq!(
            accepted(nested, vec![edit(7..18, "  #{<<B}\n  b\n  B\n")]),
            [false; 3]
        );
        let nested = "p <<~A\n#{<<~B}\nb\nB\nA\n";
        assert_eq!(
            accepted(nested, vec![edit(7..19, "  #{<<~B}\n  b\n  B\n")]),
            [true; 3]
        );
    }

    #[test]
    fn kept_middle_decomposition_checks_each_part() {
        let accepted = |source: &str, edit: Correction| {
            (0..3)
                .map(|stage| {
                    run(source, vec![edit.clone()], stage)[0]
                        .correction
                        .is_some()
                })
                .collect::<Vec<_>>()
        };
        // `SQL` repeats inside `NOSQL`: anchors are ambiguous, the kept middle
        // is not, and its parts pass the closing-line checks.
        let source = "foo(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n)\n";
        let start = source.find('\n').unwrap();
        let end = source.rfind(')').unwrap() + 1;
        let middle = &source[start..end - 2];
        let fixed = edit(start..end, &format!("){middle}"));
        assert_eq!(accepted(source, fixed), [true; 3]);
        // Joining a closing line with following code is still rejected.
        let source = "foo(<<-SQL, <<-NOSQL\n  foo\nSQL\n  bar\nNOSQL\n).x\n";
        let end = source.rfind(')').unwrap() + 1;
        let middle = &source[start..end - 2];
        let joined = edit(start..end, &format!("){middle}"));
        assert_eq!(accepted(source, joined), [false; 3]);
    }

    #[test]
    fn equivalent_quotes_are_structural_not_a_cop_name_exception() {
        for stage in 0..3 {
            for op in ["<<", "<<-", "<<~"] {
                for quote in ["'", "\""] {
                    let source = format!("p {op}{quote}EOS{quote}\ntext\nEOS\n");
                    let end = source.find('\n').unwrap();
                    let edit = edit(2..end, &format!("{op}EOS"));
                    assert!(run(&source, vec![edit], stage)[0].correction.is_some());
                }
            }
            for (source, replacement) in [
                ("p <<~'EOS'\n#{danger}\nEOS\n", "<<~EOS"),
                ("p <<~'EOS'\n\\n\nEOS\n", "<<~EOS"),
                ("p <<~' EOS '\ntext\n EOS \n", "<<~EOS"),
                ("p <<~'123'\ntext\n123\n", "<<~123"),
                ("p <<~`EOS`\ntext\nEOS\n", "<<~EOS"),
                ("p <<~'ÉOS'\ntext\nÉOS\n", "<<~ÉOS"),
            ] {
                let end = source.find('\n').unwrap();
                assert!(
                    run(source, vec![edit(2..end, replacement)], stage)[0]
                        .correction
                        .is_none(),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn structural_quote_suffix_separators_must_survive_the_union() {
        let source = "p <<~'EOS' +é\ntext\nEOS\n";
        let end = source.find(" +é").unwrap();
        for stage in 0..3 {
            let quote = edit(2..end, "<<~EOS");
            assert!(
                run(source, vec![quote.clone()], stage)[0]
                    .correction
                    .is_some()
            );
            let found = run(source, vec![quote, edit(end..end + 2, "")], stage);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn only_raw_empty_string_owners_allow_atomic_terminator_removal() {
        for stage in 0..3 {
            for source in [
                "p <<~EOS\nEOS\n",
                "p <<~'EOS' # tail\r\nEOS\r\n",
                "p <<~EOS\nEOS",
            ] {
                let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
                let parsed = ruby_prism::parse(&file.bytes);
                let h = &file.heredoc_boundaries(&parsed.node()).heredocs[0];
                let replacement = format!("\"\"{}", &source[h.opening.end..h.closing.start]);
                assert!(
                    run(
                        source,
                        vec![edit(h.opening.start..h.closing.end, &replacement)],
                        stage
                    )[0]
                    .correction
                    .is_some(),
                    "{source}"
                );
            }
            for source in [
                "p <<~EOS\n\nEOS\n",
                "p <<~EOS\n  \nEOS\n",
                "p <<~`EOS`\nEOS\n",
            ] {
                let end = source.find('\n').unwrap();
                let replacement = format!("\"\"{}", &source[end..end + 1]);
                assert!(
                    run(source, vec![edit(2..source.len(), &replacement)], stage)[0]
                        .correction
                        .is_none(),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn structural_proofs_are_lazy_main_parse_owned_and_bounded() {
        use crate::cops::style::{EmptyHeredoc, RedundantHeredocDelimiterQuotes};
        let source = "p <<~'A', <<~B\ntext\nA\nB\n".repeat(1000);
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let cops: &[&dyn Cop] = &[&EmptyHeredoc::default(), &RedundantHeredocDelimiterQuotes];
        let mut out = Vec::new();
        run_cops_with_corrections(cops, &parsed, &file, &mut out, false);
        assert_eq!(out.len(), 2000);
        assert!(file.token_ranges.get().is_none());
        assert!(out.iter().all(|o| o.correction.is_none()));
        out.clear();
        run_cops(cops, &parsed, &file, &mut out);
        let index = file.heredoc_boundaries(&parsed.node());
        assert_eq!(index.heredocs.len(), 2000);
        assert_eq!(index.equivalent_quotes.len(), 1000);
        assert!(out.iter().all(|o| o.correction.is_some()));
        assert!(Arc::ptr_eq(index, file.heredoc_boundaries(&parsed.node())));
        let source = format!("p <<~A, <<~B\n{}\nA\nB\n", "x".repeat(PROOF_BYTES));
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        out.clear();
        run_cops(&[&EmptyHeredoc::default()], &parsed, &file, &mut out);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].correction.is_none(),
            "do not copy an unbounded queued sibling"
        );
    }

    #[test]
    fn source_node_leave_and_pipeline_collection_share_the_boundary_proof() {
        let source = "p <<~\"#{1 + 2}\"\n#{1+2}\n#{1 + 2}\n";
        let plus = source.find("1+2").unwrap() + 1;
        for stage in 0..3 {
            let probe = Probe {
                stage,
                edits: vec![edit(plus..plus + 1, " + ")],
            };
            let found = run(source, probe.edits.clone(), stage);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (2, 4));
            assert_eq!(found[0].message, "boundary probe");
            assert!(found[0].correction.is_none());
            let pipeline = crate::pipeline::lint_str_with_corrections(&[&probe], source);
            assert_eq!(pipeline.len(), 1);
            assert!(pipeline[0].correction.is_none());
        }
    }

    #[test]
    fn leading_separator_removal_is_withheld_at_every_entry_and_in_atomic_envelopes() {
        let source = "def p(x = [])\n  Kernel.p(x)\n  []\nend\nA = 1\np <<A\n1\nA\n";
        let at = source.find(" <<A").unwrap();
        let corrected = source.replacen("p <<A", "p<<A", 1);
        assert_eq!(ruby_prism::parse(corrected.as_bytes()).errors().count(), 0);
        let file = SourceFile::new("test.rb".into(), corrected.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(file.heredoc_boundaries(&parsed.node()).heredocs.is_empty());
        for stage in 0..3 {
            for correction in [edit(at..at + 1, ""), edit(0..source.len(), &corrected)] {
                let probe = Probe {
                    stage,
                    edits: vec![correction],
                };
                for found in [
                    run(source, probe.edits.clone(), stage),
                    crate::pipeline::lint_str_with_corrections(&[&probe], source),
                ] {
                    assert_eq!(found.len(), 1);
                    assert_eq!(found[0].message, "boundary probe");
                    assert!(found[0].correction.is_none(), "stage {stage}");
                }
            }
        }
    }

    #[test]
    fn adjacent_leading_separator_removals_require_an_immutable_witness() {
        for whitespace in ["  ", " \t", "\x0b\x0c", "\r "] {
            let source = format!("p{whitespace}<<A\ntext\nA\n");
            let at = source.find("<<A").unwrap() - 2;
            let edits = [edit(at..at + 1, ""), edit(at + 1..at + 2, "")];
            for one in &edits {
                assert!(run(&source, vec![one.clone()], 0)[0].correction.is_some());
            }
            for stage in 0..3 {
                for atomic in [false, true] {
                    let edits = if atomic {
                        vec![
                            edit(0..at + 1, &source[..at]),
                            edit(at + 1..source.len(), &source[at + 2..]),
                        ]
                    } else {
                        edits.to_vec()
                    };
                    for edits in [edits.clone(), edits.into_iter().rev().collect()] {
                        let found = run(&source, edits, stage);
                        assert!(
                            found.iter().any(|o| o.correction.is_none()),
                            "{whitespace:?}"
                        );
                    }
                }
            }
        }
        // A write to the exposed witness is unsafe even when it is not itself
        // adjacent to the opening; a newly inserted '<' is unsafe too.
        let source = "p  <<A\ntext\nA\n";
        let found = run(source, vec![edit(1..2, "x"), edit(2..3, "")], 0);
        assert!(found[1].correction.is_none());
        assert!(
            run(source, vec![edit(3..3, "<")], 0)[0]
                .correction
                .is_none()
        );
        for source in ["p( <<A)\ntext\nA\n", "p [ <<A]\ntext\nA\n"] {
            let at = source.find(" <<A").unwrap();
            assert!(
                run(source, vec![edit(at..at + 1, "")], 0)[0]
                    .correction
                    .is_some()
            );
        }
        // Immutable separators inside atomic envelopes remain valid witnesses.
        for correction in [
            edit(1..2, ""),
            edit(2..3, ""),
            edit(0..source.len(), &source.replace("p  ", "p ")),
        ] {
            assert!(run(source, vec![correction], 0)[0].correction.is_some());
        }
    }

    #[test]
    fn multibyte_opener_suffixes_are_withheld_at_every_public_entry() {
        for op in ["<<", "<<-", "<<~"] {
            for suffix in ["é", "中", "💎", "\u{80}"] {
                let source = format!("A = 1\np {op}A\ntext\nA{suffix}\nA\n");
                let end = source.find("A\ntext").unwrap() + 1;
                for stage in 0..3 {
                    let probe = Probe {
                        stage,
                        edits: vec![edit(end..end, suffix)],
                    };
                    for found in [
                        run(&source, probe.edits.clone(), stage),
                        crate::pipeline::lint_str_with_corrections(&[&probe], &source),
                    ] {
                        assert_eq!(found.len(), 1);
                        assert_eq!((found[0].line, found[0].col), (2, 4 + op.len() as u32));
                        assert_eq!(found[0].message, "boundary probe");
                        assert!(found[0].correction.is_none(), "{op} {suffix} stage {stage}");
                    }
                }
            }
        }
    }

    #[test]
    fn removal_cannot_expose_a_multibyte_opener_suffix() {
        let source = "A = 1\né = \"\"\np <<A +é\ntext\nAé\nA\n";
        let end = source.find("A +é").unwrap() + 1;
        for stage in 0..3 {
            let found = run(source, vec![edit(end..end + 2, "")], stage);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn atomic_envelopes_cannot_extend_openers_or_expose_multibyte_suffixes() {
        for (source, corrected) in [
            (
                "A = 1\np <<A\ntext\nAé\nA\n",
                "A = 1\np <<Aé\ntext\nAé\nA\n",
            ),
            (
                "A = 1\né = \"\"\np <<A +é\ntext\nAé\nA\n",
                "A = 1\né = \"\"\np <<Aé\ntext\nAé\nA\n",
            ),
        ] {
            assert_eq!(ruby_prism::parse(corrected.as_bytes()).errors().count(), 0);
            for stage in 0..3 {
                for end in [source.find("\ntext").unwrap(), source.len()] {
                    let new_end = if end == source.len() {
                        corrected.len()
                    } else {
                        corrected.find("\ntext").unwrap()
                    };
                    let found = run(source, vec![edit(0..end, &corrected[..new_end])], stage);
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn adjacent_suffix_removals_need_an_immutable_name_separator() {
        for suffix in ["é", "z"] {
            let source = format!("A = 1\n{suffix} = \"\"\np <<A + {suffix}\ntext\nA{suffix}\nA\n");
            let end = source.find("A + ").unwrap() + 1;
            let edits = [edit(end..end + 2, ""), edit(end + 2..end + 3, "")];
            for one in &edits {
                assert!(run(&source, vec![one.clone()], 0)[0].correction.is_some());
            }
            for stage in 0..3 {
                for edits in [edits.to_vec(), edits.iter().rev().cloned().collect()] {
                    for atomic in [false, true] {
                        let edits = if atomic {
                            edits
                                .iter()
                                .map(|e| {
                                    let header_end = source.find("\ntext").unwrap();
                                    // Adjacent, disjoint atomic envelopes: the
                                    // first retains the parsed opening anchor;
                                    // the second retains the encoding suffix.
                                    if e.range.start == end {
                                        edit(0..e.range.end, &source[..e.range.start])
                                    } else {
                                        edit(
                                            e.range.start..header_end,
                                            &source[e.range.end..header_end],
                                        )
                                    }
                                })
                                .collect()
                        } else {
                            edits.clone()
                        };
                        let found = run(&source, edits, stage);
                        assert!(found[0].correction.is_none() || found[1].correction.is_none());
                    }
                }
            }
        }
    }

    #[test]
    fn legacy_name_suffix_uncertainty_is_withheld_without_decoding() {
        // Correction strings are UTF-8, but the source can use single/multibyte
        // legacy labels. Deletion exposes actual encoding-native bytes.
        for (encoding, suffix) in [
            ("ISO-8859-1", &b"\xe9"[..]),
            ("Windows-31J", &b"\x82\xa0"[..]),
            ("ASCII-8BIT", &b"\xff"[..]),
        ] {
            let mut source = format!("# coding: {encoding}\nA = 1\np <<A +").into_bytes();
            let end = source.len() - 2;
            source.extend_from_slice(suffix);
            source.extend_from_slice(b"\ntext\nA");
            source.extend_from_slice(suffix);
            source.extend_from_slice(b"\nA\n");
            let file = SourceFile::new("test.rb".into(), source);
            let parsed = ruby_prism::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0, "{encoding}");
            for stage in 0..3 {
                for correction in [edit(end..end, "é"), edit(end..end + 2, "")] {
                    let probe = Probe {
                        stage,
                        edits: vec![correction],
                    };
                    let mut found = Vec::new();
                    assert!(run_cops(&[&probe], &parsed, &file, &mut found).is_empty());
                    assert_eq!(found.len(), 1);
                    assert!(found[0].correction.is_none(), "{encoding} stage {stage}");
                }
            }
        }
    }

    #[test]
    fn ordinary_header_and_atomic_spacing_controls_remain_proved() {
        let source = "p <<A  + \"\"\ntext\nA\np 1\n";
        let end = source.find("A  +").unwrap() + 1;
        for correction in [
            edit(end..end + 1, ""),
            edit(0..source.len(), &source.replace("A  +", "A +")),
            edit(end..end, " "),
        ] {
            assert!(run(source, vec![correction], 0)[0].correction.is_some());
        }
    }

    #[test]
    fn adjacent_conflicting_and_selected_subsets_cannot_remove_all_witnesses() {
        let source = "p <<~'abc'\naxbyc\nabc\n";
        let x = source.find("axbyc").unwrap() + 1;
        let edits = [edit(x..x + 1, ""), edit(x + 2..x + 3, "")];
        for one in &edits {
            assert!(run(source, vec![one.clone()], 0)[0].correction.is_some());
        }
        let mut combined = edits.to_vec();
        combined.push(edit(x..x + 3, "abc")); // conflicting safe-looking peer
        for reverse in [false, true] {
            if reverse {
                combined.reverse();
            }
            let found = run(source, combined.clone(), 0);
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
        let source = "p <<~\"#{1 + 2}\"\n#{1+2}\n#{1 + 2}\n";
        let plus = source.find("1+2").unwrap() + 1;
        let found = run(
            source,
            vec![edit(plus..plus, " "), edit(plus + 1..plus + 1, " ")],
            2,
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn name_separator_bytes_match_prism_or_remain_encoding_uncertain() {
        for byte in 0..=u8::MAX {
            assert_eq!(
                could_continue_name(byte),
                byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80,
                "{byte:#x}"
            );
            if could_continue_name(byte) {
                // ASCII-8BIT gives every high byte a valid single-byte width.
                // Assert actual main-parse locations, not Unicode categories.
                let mut bytes = b"# coding: ASCII-8BIT\np <<A".to_vec();
                bytes.push(byte);
                let end = bytes.len();
                bytes.extend_from_slice(b"\ntext\nA");
                bytes.push(byte);
                bytes.push(b'\n');
                let file = SourceFile::new("test.rb".into(), bytes);
                let parsed = ruby_prism::parse(&file.bytes);
                assert_eq!(parsed.errors().count(), 0, "{byte:#x}");
                let index = file.heredoc_boundaries(&parsed.node());
                assert_eq!(index.heredocs[0].opening.end, end, "{byte:#x}");
            }
        }
    }

    #[test]
    fn indentation_witnesses_match_prism_inline_whitespace() {
        for byte in 0..=u8::MAX {
            let inline = matches!(byte, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r');
            assert_eq!(
                could_be_label(&[byte, b'a'], b"a", true),
                inline,
                "{byte:#x}"
            );
            // Plain heredocs do not permit any indentation before the label.
            assert!(!could_be_label(&[byte, b'a'], b"a", false), "{byte:#x}");
        }
    }

    #[test]
    fn quoted_label_whitespace_is_not_trimmed_and_body_newlines_are_uncertain() {
        for (label, indented) in [(" abc ", false), ("#{1 + 2}", true), ("é word", true)] {
            let op = if indented { "<<~" } else { "<<" };
            let source = format!("p {op}'{label}'\n{label}x\n{label}\n");
            let x = source.find(&format!("{label}x")).unwrap() + label.len();
            assert!(
                run(&source, vec![edit(x..x + 1, "")], 0)[0]
                    .correction
                    .is_none()
            );
        }
        let source = "p <<~\"#{1 + 2}\"\n#{1+2}\n#{1 + 2}\n";
        let plus = source.find("1+2").unwrap() + 1;
        assert!(
            run(source, vec![edit(plus..plus + 1, "+\n")], 0)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn inserted_newline_groups_prove_every_resulting_line_before_composing() {
        let source = "p <<~\"#{1}\"\n#{1}#{2}\n#{1}\n";
        let split = source.find("#{1}#{2}").unwrap() + 4;
        assert!(
            run(source, vec![edit(split..split, "\n")], 0)[0]
                .correction
                .is_none()
        );
        let source = "p <<~TEXT\n#{def f\n  1 end} tail\nTEXT\n";
        let split = source.find(" end").unwrap();
        assert!(
            run(source, vec![edit(split..split + 1, "\n ")], 0)[0]
                .correction
                .is_some()
        );
        // Both immutable halves are necessary: a peer removing the suffix
        // witness must prevent the split, regardless of conflict winners.
        let end = source.find("} tail").unwrap() + 6;
        let found = run(
            source,
            vec![edit(split..split + 1, "\n "), edit(split + 1..end, "TEXT")],
            0,
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn bounded_envelopes_preserve_anchors_and_reject_ambiguous_or_large_proofs() {
        let source = "p <<~X\n#{a+a}\nX\n";
        let start = source.find("#{a+a}").unwrap();
        let found = run(source, vec![edit(start..start + 6, "#{b+b}")], 1);
        assert!(found[0].correction.is_some());
        let source = format!("p <<~X\n#{{{}}}\nX\n", "a".repeat(PROOF_BYTES));
        let start = source.find("#{").unwrap() + 2;
        assert!(
            run(&source, vec![edit(start..start + 1, "b")], 0)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn atomic_header_proof_budget_still_has_a_4096_byte_boundary() {
        for length in [PROOF_BYTES, PROOF_BYTES + 1] {
            let header = format!("x = 1;{}p <<A", " ".repeat(length - 11));
            assert_eq!(header.len(), length);
            let source = format!("{header}\ntext\nA\n");
            let replacement = header.replacen("x = 1", "x = 2", 1);
            let found = run(&source, vec![edit(0..header.len(), &replacement)], 0);
            assert_eq!(found[0].correction.is_some(), length == PROOF_BYTES);
        }
    }

    #[test]
    fn closing_eol_transfer_requires_an_untouched_dependency_in_every_subset() {
        let source = "p (<<~X\ntext\nX\n)\n";
        let closing_eol = source.find("X\n)").unwrap() + 1;
        let after = source.find(")\n").unwrap() + 1;
        let transfer = edit(closing_eol..after, "");
        assert!(
            run(source, vec![transfer.clone()], 0)[0]
                .correction
                .is_some()
        );
        let found = run(source, vec![transfer, edit(after..after + 1, "")], 0);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn empty_quoted_labels_have_no_text_anchor_but_keep_physical_boundaries() {
        let source = "p <<~\"\"\n#{1+2}\n\n";
        let found = run(
            source,
            vec![edit(0..source.len(), &source.replace("1+2", "1 + 2"))],
            1,
        );
        assert!(found[0].correction.is_some());
        let body = source.find("#{1+2}").unwrap();
        assert!(
            run(source, vec![edit(body..body + 6, "")], 0)[0]
                .correction
                .is_none()
        );
        assert!(
            run(source, vec![edit(source.len() - 1..source.len(), "")], 0)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn bounded_physical_line_decomposition_is_byte_exact() {
        let mut strings = vec![Vec::new()];
        for _ in 0..4 {
            for prefix in strings.clone() {
                for b in b"ab\n" {
                    let mut s = prefix.clone();
                    s.push(*b);
                    strings.push(s);
                }
            }
        }
        strings.sort();
        strings.dedup();
        for before in &strings {
            for after in &strings {
                let mut changes = Vec::new();
                line_changes(before, 0..before.len(), after, &mut changes);
                let mut reconstructed = Vec::new();
                let mut cursor = 0;
                for change in changes {
                    reconstructed.extend_from_slice(&before[cursor..change.range.start]);
                    reconstructed.extend_from_slice(change.replacement);
                    cursor = change.range.end;
                }
                reconstructed.extend_from_slice(&before[cursor..]);
                assert_eq!(&reconstructed, after, "{before:?} -> {after:?}");
            }
        }
    }

    #[test]
    fn physical_ownership_includes_leading_blank_lines_and_deferred_siblings() {
        for source in [
            "p <<~X\n\n  #{1}\nX\n",
            "p <<~X\n\\\n#{1}\nX\n",
            "p <<~X, <<~Y\n\n#{1}\nX\n\n#{2}\nY\n",
        ] {
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let parsed = ruby_prism::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0);
            let index = file.heredoc_boundaries(&parsed.node());
            let mut bodies: Vec<_> = index.heredocs.iter().collect();
            bodies.sort_by_key(|h| h.opening.start);
            let mut previous_end = source.find('\n').unwrap() + 1;
            for h in bodies {
                assert_eq!(h.body.start, previous_end, "{source}");
                previous_end = h.closing.end;
            }
        }
    }

    #[test]
    fn boundary_index_is_lazy_shared_and_linear_in_main_ast_ownership() {
        let source = "p <<~OUT, <<~IN\n#{1+2}\nOUT\n#{3+4}\nIN\n".repeat(1000);
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let probe = Probe {
            stage: 0,
            edits: vec![edit(
                source.find("1+2").unwrap() + 1..source.find("1+2").unwrap() + 2,
                " + ",
            )],
        };
        let mut out = Vec::new();
        run_cops_with_corrections(&[&probe], &parsed, &file, &mut out, false);
        assert!(file.token_ranges.get().is_none());
        file.literal_token_ranges(&parsed.node());
        assert!(
            file.token_ranges
                .get()
                .unwrap()
                .heredoc_boundaries
                .get()
                .is_none(),
            "detection's token metadata does not build the correction-only index"
        );
        out.clear();
        run_cops(&[&probe], &parsed, &file, &mut out);
        let index = file.heredoc_boundaries(&parsed.node());
        assert_eq!(index.heredocs.len(), 2000);
        assert_eq!(index.segments.len(), 2000);
        assert!(index.segments.iter().all(|s| s.owners.len() == 1));
        assert!(Arc::ptr_eq(index, file.heredoc_boundaries(&parsed.node())));
        assert!(Arc::ptr_eq(&index.line_starts, &file.line_starts));
        assert!(out[0].correction.is_some());
    }
}
