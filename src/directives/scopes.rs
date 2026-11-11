//! Suppression analysis ported from RuboCop's CommentConfig, DisableNext and
//! PushPop (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use super::{
    comment_attachments,
    comments::{self, DirectiveComment, signed_hits},
    statement_ends,
};
use crate::source::SourceFile;
use ruby_prism::ParseResult;

type Span = (u32, u32);

/// Unmerged interval with its actual opening source directive (None for config).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DisabledRange<'a> {
    pub first: u32,
    pub last: u32,
    pub directive: Option<&'a DirectiveComment>,
}

/// Unmerged intervals matter to CommentsHelp (adjacent disables are distinct).
#[derive(Debug)]
struct CommentRanges {
    spans: Vec<Span>,
    prefix_ends: Vec<u32>,
}

/// Cached directive operations and lazily normalized disabled intervals per cop.
/// Analysis is linear in directive arguments per queried cop, not source lines
/// per offense; nested pushes store one open-state value, not copies of history.
#[derive(Debug, Default)]
pub struct Directives {
    records: Vec<DirectiveComment>,
    /// CommentsHelp observes lexical CommentConfig ranges, including block
    /// pseudo-directives. These must never suppress code or opt cops in.
    comment_records: Vec<DirectiveComment>,
    bounds: HashMap<u32, Span>,
    initially_disabled: HashSet<String>,
    ranges: Mutex<HashMap<String, Vec<Span>>>,
    comment_ranges: Mutex<HashMap<String, CommentRanges>>,
}

impl Directives {
    /// Extract directives using the main parse and the shared lexical records.
    pub fn parse(result: &ParseResult<'_>, file: &SourceFile) -> Self {
        Self::parse_with_disabled(result, file, &[])
    }

    pub(crate) fn parse_with_disabled(
        result: &ParseResult<'_>,
        file: &SourceFile,
        disabled: &[&str],
    ) -> Self {
        file.record_comments(result);
        // Preserve the existing suppression grammar: only # comments, and
        // signed targets only for push/next (Style cops still see all records).
        let records: Vec<_> = comments::suppression_records(file).cloned().collect();
        let comment_records = file.directive_records().to_vec();
        let needs_bounds = comment_records
            .iter()
            .any(|d| d.standalone && is_next(d.mode) && d.has_cops());
        let mut bounds = HashMap::new();
        if needs_bounds {
            let attachments = comment_attachments(file);
            let ends: HashMap<_, _> = statement_ends(
                &result.node(),
                file,
                comment_records
                    .iter()
                    .filter(|d| d.standalone && is_next(d.mode) && d.has_cops())
                    .filter_map(|d| attachments.get(&d.line).copied())
                    .collect(),
            )
            .into_iter()
            .collect();
            for d in &comment_records {
                if d.standalone
                    && is_next(d.mode)
                    && let Some(&line) = attachments.get(&d.line)
                    && let Some(&end) = ends.get(&line)
                {
                    bounds.insert(d.line, (line, end));
                }
            }
        }
        Self {
            records,
            comment_records,
            bounds,
            initially_disabled: disabled.iter().map(|s| (*s).to_owned()).collect(),
            ranges: Mutex::default(),
            comment_ranges: Mutex::default(),
        }
    }

    /// Source opt-ins mobilize disabled cops, but their initial disabled range
    /// remains in effect outside the enabling scope. Plain enable does not
    /// expand departments/all; enable-next expands departments but not all.
    pub(crate) fn opts_in(&self, cop: &str) -> bool {
        source_opts_in(self.records.iter(), cop)
    }

    /// Whether `cop` may report an offense on `line`.
    pub fn is_enabled(&self, cop: &str, line: u32) -> bool {
        self.is_enabled_for_lines(cop, line, line)
    }

    /// A disabled line anywhere in a highlight or correction span suppresses it.
    /// Binary search intervals, never scan the lines in an arbitrarily long span.
    pub fn is_enabled_for_lines(&self, cop: &str, first: u32, last: u32) -> bool {
        debug_assert!(first <= last);
        let mut cache = self.ranges.lock().expect("directive ranges");
        let ranges = cache
            .entry(cop.to_owned())
            .or_insert_with(|| self.analyze(cop));
        let i = ranges.partition_point(|&(_, end)| end < first);
        ranges.get(i).is_none_or(|&(start, _)| start > last)
    }

    /// CommentsHelp distinguishes containing / contained disabled intervals
    /// from a merely overlapping interval. `lines` is physical, end-exclusive.
    pub(crate) fn contains_disables(&self, cop: &str, lines: std::ops::Range<u32>) -> bool {
        if lines.is_empty() {
            return false;
        }
        let last = lines.end - 1;
        let mut cache = self
            .comment_ranges
            .lock()
            .expect("comment directive ranges");
        let ranges = cache.entry(cop.to_owned()).or_insert_with(|| {
            let mut spans = self.analyze_unmerged(cop, &self.comment_records);
            spans.sort_unstable();
            let mut max_end = 0;
            let prefix_ends = spans
                .iter()
                .map(|&(_, end)| {
                    max_end = max_end.max(end);
                    max_end
                })
                .collect();
            CommentRanges { spans, prefix_ends }
        });
        let covering = ranges
            .spans
            .partition_point(|&(start, _)| start <= lines.start);
        if covering > 0 && ranges.prefix_ends[covering - 1] >= last {
            return true;
        }
        let first = ranges
            .spans
            .partition_point(|&(start, _)| start < lines.start);
        ranges.spans[first..]
            .iter()
            .take_while(|&&(start, _)| start <= last)
            .any(|&(_, end)| end <= last)
    }

    /// Whether there are no effective suppression operations.
    pub fn is_empty(&self) -> bool {
        self.initially_disabled.is_empty()
            && !self.records.iter().any(|d| match d.mode {
                "push" => !d.signed.is_empty(),
                "pop" => false,
                mode if is_next(mode) => d.has_cops() && self.bounds.contains_key(&d.line),
                "disable" | "todo" => !d.names.is_empty(),
                "enable" => !d.names.is_empty() && d.standalone && !d.embedded,
                _ => false,
            })
    }

    pub(crate) fn records(&self) -> &[DirectiveComment] {
        &self.records
    }

    pub(crate) fn statement_bounds(&self, directive: &DirectiveComment) -> Option<Span> {
        self.bounds.get(&directive.line).copied()
    }

    pub(crate) fn attached(&self, directive: &DirectiveComment) -> bool {
        self.statement_bounds(directive).is_some()
    }

    pub(crate) fn disabled_ranges(&self, cop: &str) -> Vec<DisabledRange<'_>> {
        self.attributed_ranges(cop, &self.records)
    }

    fn analyze_unmerged(&self, cop: &str, records: &[DirectiveComment]) -> Vec<Span> {
        self.attributed_ranges(cop, records)
            .into_iter()
            .map(|r| (r.first, r.last))
            .collect()
    }

    fn attributed_ranges<'a>(
        &self,
        cop: &str,
        records: &'a [DirectiveComment],
    ) -> Vec<DisabledRange<'a>> {
        let mut ranges = Vec::new();
        let mut open = self.initially_disabled.contains(cop).then_some((1, None));
        let mut stack: Vec<Option<(u32, Option<&DirectiveComment>)>> = Vec::new();
        for d in records {
            if d.mode == "pop" {
                if let Some(saved) = stack.pop() {
                    close(&mut ranges, open, d.line.saturating_sub(1));
                    open = saved.map(|(_, directive)| (d.line, directive));
                }
                continue;
            }
            if d.mode == "push" {
                stack.push(open);
            }
            if matches!(d.mode, "push" | "next") {
                // Ruby groups operations by sign, preserving the order in which
                // each sign first appears, rather than processing alternating args.
                let first = d.signed.first().map(|&(plus, _)| plus);
                for plus in first.into_iter().chain(first.map(|p| !p)) {
                    // Immunity ignores source disables, not the initial disabled
                    // state or push/pop boundaries of a source-only opt-in.
                    if cop == comments::BAN && !plus {
                        continue;
                    }
                    if !d
                        .signed
                        .iter()
                        .any(|(p, name)| *p == plus && signed_hits(name, cop))
                    {
                        continue;
                    }
                    if d.mode == "push" {
                        if plus {
                            close(&mut ranges, open, d.line);
                            open = None;
                        } else if open.is_none() {
                            open = Some((d.line, Some(d)));
                        }
                    } else if let Some(&(start, end)) = self.bounds.get(&d.line) {
                        if plus {
                            suspend(&mut ranges, &mut open, start, end);
                        } else {
                            ranges.push(DisabledRange {
                                first: start,
                                last: end,
                                directive: Some(d),
                            });
                        }
                    }
                }
                continue;
            }
            if !d.names.iter().any(|name| {
                // all/Lint omit this lifecycle cop; explicit names still work.
                !(cop == "Lint/RedundantCopDisableDirective"
                    && matches!(name.as_str(), "all" | "Lint"))
                    && comments::hits(name, cop)
            }) {
                continue;
            }
            match d.mode {
                "disable" | "todo" | "disable-next" | "todo-next" if cop == comments::BAN => {}
                "disable-next" | "todo-next" => {
                    if let Some(&bounds) = self.bounds.get(&d.line) {
                        ranges.push(DisabledRange {
                            first: bounds.0,
                            last: bounds.1,
                            directive: Some(d),
                        });
                    }
                }
                "enable-next" => {
                    if let Some(&(start, end)) = self.bounds.get(&d.line) {
                        suspend(&mut ranges, &mut open, start, end);
                    }
                }
                "disable" | "todo" if !d.standalone || d.embedded => ranges.push(DisabledRange {
                    first: d.line,
                    last: d.line,
                    directive: Some(d),
                }),
                "disable" | "todo" => {
                    close(&mut ranges, open, d.line);
                    open = Some((d.line, Some(d)));
                }
                "enable" if d.standalone && !d.embedded => {
                    close(&mut ranges, open, d.line);
                    open = None;
                }
                _ => {}
            }
        }
        close(&mut ranges, open, u32::MAX);
        ranges
    }

    fn analyze(&self, cop: &str) -> Vec<Span> {
        let mut ranges = self.analyze_unmerged(cop, &self.records);
        ranges.sort_unstable();
        let mut merged: Vec<Span> = Vec::new();
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut()
                && start <= last.1.saturating_add(1)
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        merged
    }
}

pub(crate) fn source_opts_in<'a>(
    records: impl IntoIterator<Item = &'a DirectiveComment>,
    cop: &str,
) -> bool {
    records.into_iter().any(|d| match d.mode {
        "push" | "next" => d
            .signed
            .iter()
            .any(|(plus, name)| *plus && signed_hits(name, cop)),
        "enable-next" => d
            .names
            .iter()
            .any(|name| name != "all" && comments::hits(name, cop)),
        "enable" => d
            .names
            .iter()
            .any(|name| name == cop || Some(name.as_str()) == cop.rsplit('/').next()),
        _ => false,
    })
}

/// Permanent enables whose removal leaves the effective open state unchanged.
/// Next-statement scopes restore that state; push/pop must snapshot it even
/// when their arguments target another cop. This proof is separate from the
/// upstream pairing counters used for diagnostics.
pub(crate) fn redundant_enables(file: &SourceFile, cop: &str) -> HashSet<u32> {
    let mut disabled = file.directive_disabled().iter().any(|n| n == cop);
    let mut stack = Vec::new();
    let mut redundant = HashSet::new();
    for d in file.directive_records() {
        if file.bytes[d.range.start] != b'#' {
            continue;
        }
        match d.mode {
            "pop" => {
                if let Some(saved) = stack.pop() {
                    disabled = saved;
                }
            }
            "push" => {
                stack.push(disabled);
                let first = d.signed.first().map(|&(plus, _)| plus);
                for plus in first.into_iter().chain(first.map(|p| !p)) {
                    if d.signed
                        .iter()
                        .any(|(p, n)| *p == plus && signed_hits(n, cop))
                    {
                        disabled = !plus;
                    }
                }
            }
            "disable" | "todo" | "enable"
                if d.standalone
                    && !d.embedded
                    && d.signed.is_empty()
                    && d.names.iter().any(|n| comments::hits(n, cop)) =>
            {
                if d.mode == "enable" && !disabled {
                    redundant.insert(d.line);
                }
                disabled = d.mode != "enable";
            }
            _ => {}
        }
    }
    redundant
}

fn is_next(mode: &str) -> bool {
    matches!(mode, "disable-next" | "todo-next" | "enable-next" | "next")
}

fn close<'a>(
    ranges: &mut Vec<DisabledRange<'a>>,
    open: Option<(u32, Option<&'a DirectiveComment>)>,
    end: u32,
) {
    if let Some((start, directive)) = open
        && start <= end
    {
        ranges.push(DisabledRange {
            first: start,
            last: end,
            directive,
        });
    }
}
fn suspend<'a>(
    ranges: &mut Vec<DisabledRange<'a>>,
    open: &mut Option<(u32, Option<&'a DirectiveComment>)>,
    start: u32,
    end: u32,
) {
    if let Some((_, directive)) = *open {
        close(ranges, *open, start.saturating_sub(1));
        *open = Some((end.saturating_add(1), directive));
    }
}
