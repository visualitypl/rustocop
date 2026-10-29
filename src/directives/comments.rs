//! Shared lexical directive facts for the Style directive cops. Matching is
//! ported from RuboCop's DirectiveComment (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov); rustocop also accepts its own marker spelling.
use crate::{cops::Registry, source::SourceFile};
use std::ops::Range;

pub(crate) const BAN: &str = "Style/DisableCopsWithinSourceCodeDirective";

#[derive(Debug, Clone)]
pub(crate) struct DirectiveComment {
    pub range: Range<usize>,
    pub line: u32,
    pub mode: &'static str,
    pub names: Vec<String>,
    pub signed: Vec<(bool, String)>,
    pub standalone: bool,
    pub embedded: bool,
    pub pop: Option<usize>,
}
impl DirectiveComment {
    pub fn disabled(&self) -> bool {
        matches!(self.mode, "disable" | "todo")
    }
    pub fn same_names(&self, other: &Self) -> bool {
        let mut a: Vec<_> = self.names.iter().collect();
        let mut b: Vec<_> = other.names.iter().collect();
        a.sort_unstable();
        b.sort_unstable();
        a == b
    }
    pub fn replacement_mode(&self) -> &'static str {
        match (
            self.signed.iter().any(|(enable, _)| *enable),
            self.signed.iter().any(|(enable, _)| !enable),
        ) {
            (true, true) => "next",
            (true, false) => "enable-next",
            _ => "disable-next",
        }
    }
    pub fn has_cops(&self) -> bool {
        if matches!(self.mode, "push" | "next") {
            !self.signed.is_empty()
        } else {
            !self.names.is_empty()
        }
    }
}

pub(crate) fn collect(file: &SourceFile) -> Vec<DirectiveComment> {
    let mut records: Vec<DirectiveComment> = Vec::new();
    let mut pushes = Vec::new();
    for range in file.comments() {
        let text = &file.bytes[range.clone()];
        let Some((hash, _, rest)) = super::find_marker(text) else {
            continue;
        };
        if super::is_commented_out(&text[..hash]) {
            continue;
        }
        let header = &text[..text.len() - rest.len()];
        let Some((spelling, _)) = super::MODES.iter().find(|(s, _)| header.ends_with(s)) else {
            continue;
        };
        let mode = std::str::from_utf8(spelling).expect("ASCII modes");
        let mut names = Vec::new();
        let mut signed = Vec::new();
        if let Some(targets) = super::parse_targets(rest) {
            names = match targets {
                super::Targets::All => vec!["all".into()],
                super::Targets::List(names) => names,
            };
        } else if super::skip_space(rest).len() != rest.len() {
            let mut tail = super::skip_space(rest);
            while let Some((&sign, after)) = tail.split_first() {
                if !matches!(sign, b'+' | b'-') {
                    break;
                }
                let Some(len) = super::cop_name_len(after) else {
                    break;
                };
                signed.push((
                    sign == b'+',
                    String::from_utf8_lossy(&after[..len]).into_owned(),
                ));
                let remaining = &after[len..];
                tail = super::skip_space(remaining);
                if tail.len() == remaining.len() {
                    break;
                }
            }
            // Ordinary forms capture signed text too, but it is not a valid
            // range target. The ban nevertheless detects that captured text.
            if !matches!(mode, "push" | "next") {
                names = signed
                    .iter()
                    .map(|(plus, name)| format!("{}{name}", if *plus { '+' } else { '-' }))
                    .collect();
            }
        }
        let line = file.line_of(range.start);
        let record = DirectiveComment {
            range: range.clone(),
            line,
            mode,
            names,
            signed,
            standalone: super::starts_line(file, range.start),
            embedded: hash != 0,
            pop: None,
        };
        if mode == "push" {
            pushes.push(records.len());
        }
        if mode == "pop"
            && let Some(push) = pushes.pop()
        {
            records[push].pop = Some(records.len());
        }
        records.push(record);
    }
    records
}

/// Directive records honored by suppression and source opt-in analysis.
/// Lexical consumers may also inspect block comments and malformed signed
/// targets; those must not mobilize configured-disabled cops.
pub(crate) fn suppression_records(file: &SourceFile) -> impl Iterator<Item = &DirectiveComment> {
    file.directive_records().iter().filter(|d| {
        file.bytes[d.range.start] == b'#'
            && (matches!(d.mode, "push" | "pop" | "next")
                || !d.names.is_empty() && d.signed.is_empty())
    })
}

/// A bounded, registry-sized target vocabulary, independent of source length.
/// Unknown qualified names are also kept verbatim by the scope analyzer.
pub(crate) fn registered_names() -> &'static [&'static str] {
    static NAMES: std::sync::LazyLock<Vec<&'static str>> =
        std::sync::LazyLock::new(|| Registry::new().cops().iter().map(|c| c.name()).collect());
    &NAMES
}
pub(crate) fn signed_hits(name: &str, cop: &str) -> bool {
    // The pinned reference expands departments for signed args, not `all`.
    (name != "all" || cop == "all") && hits(name, cop)
}
pub(crate) fn hits(name: &str, cop: &str) -> bool {
    name == "all"
        || name == cop
        || Some(name) == cop.split('/').next()
        || Some(name) == cop.rsplit('/').next()
}
