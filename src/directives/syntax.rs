//! Strict diagnostic view of the existing permissive directive grammar.
//! The scope parser intentionally honors the first captured prefix even when
//! trailing prose is malformed; validation must not change that behavior.
//! Follows RuboCop's DirectiveComment (MIT, Copyright (c) 2012-26
//! Bozhidar Batsov), with both accepted project prefixes.
use super::{MODES, cop_name_len, find_marker, skip_space};

pub(crate) const MODES_MESSAGE: &str = "The mode name must be one of `enable`, `disable`, `disable-next`, `enable-next`, `todo`, `todo-next`, `next`, `push`, or `pop`.";

pub(crate) fn marker_end(text: &[u8]) -> Option<usize> {
    let tail = skip_space(text.strip_prefix(b"#")?);
    let tail = tail
        .strip_prefix(b"rubocop")
        .or_else(|| tail.strip_prefix(b"rustocop"))?;
    let tail = skip_space(skip_space(tail).strip_prefix(b":")?);
    Some(text.len() - tail.len())
}

#[derive(Debug)]
pub(crate) struct Match<'a> {
    pub start: usize,
    pub end: usize,
    pub mode: &'static str,
    pub names: Vec<&'a str>,
    pub signed: bool,
}

pub(crate) fn matches(text: &[u8]) -> Vec<Match<'_>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some((hash, _, rest)) = find_marker(&text[offset..]) {
        let header_end = text.len() - rest.len();
        let header = &text[..header_end];
        let (spelling, _) = MODES.iter().find(|(s, _)| header.ends_with(s)).unwrap();
        let mode = std::str::from_utf8(spelling).unwrap();
        let mut tail = skip_space(rest);
        let mut names = Vec::new();
        let mut signed = false;
        if tail.len() != rest.len() {
            if tail.starts_with(b"all") {
                names.push("all");
                tail = &tail[3..];
            } else if cop_name_len(tail).is_some() {
                loop {
                    let len = cop_name_len(tail).unwrap();
                    names.push(std::str::from_utf8(&tail[..len]).unwrap());
                    tail = &tail[len..];
                    let Some(next) = skip_space(tail).strip_prefix(b",") else {
                        break;
                    };
                    let next = skip_space(next);
                    if cop_name_len(next).is_none() {
                        break;
                    }
                    tail = next;
                }
            } else if matches!(tail.first(), Some(b'+' | b'-'))
                && cop_name_len(&tail[1..]).is_some()
            {
                signed = true;
                loop {
                    let len = 1 + cop_name_len(&tail[1..]).unwrap();
                    names.push(std::str::from_utf8(&tail[..len]).unwrap());
                    tail = &tail[len..];
                    let next = skip_space(tail);
                    if next.len() == tail.len()
                        || !matches!(next.first(), Some(b'+' | b'-'))
                        || cop_name_len(&next[1..]).is_none()
                    {
                        break;
                    }
                    tail = next;
                }
            } else {
                tail = rest;
            }
        } else {
            tail = rest;
        }
        let end = text.len() - tail.len();
        found.push(Match {
            start: offset + hash,
            end,
            mode,
            names,
            signed,
        });
        offset = end;
    }
    found
}
