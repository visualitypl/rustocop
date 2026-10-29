//! Deterministic, single-pass byte edits against one source snapshot.

use anyhow::{Result, ensure};

use crate::offense::{Correction, Offense};

/// The corrected bytes and number of replacements actually applied.
#[derive(Debug)]
pub struct AppliedCorrections {
    /// Source after the selected edits, preserving all untouched bytes.
    pub bytes: Vec<u8>,
    /// Number of non-overlapping, non-noop corrections applied.
    pub count: usize,
}

/// Apply corrections already filtered for safety and directives by the caller.
/// Ranges refer to the original bytes, not to intermediate results. Conflicts
/// are resolved by (start, end, cop name, replacement, message), first wins.
/// Adjacent replacements are allowed; insertions on either boundary of a
/// replacement conflict, as do insertions at the same offset. Invalid ranges
/// reject the entire batch without modifying the source. Raw bytes are used
/// deliberately: Ruby source need not be UTF-8.
pub fn apply(source: &[u8], offenses: &[Offense]) -> Result<AppliedCorrections> {
    let mut candidates = Vec::new();
    for offense in offenses {
        let Some(correction) = &offense.correction else {
            continue;
        };
        let range = &correction.range;
        ensure!(
            range.start <= range.end && range.end <= source.len(),
            "invalid correction range {}..{} from {} (source has {} bytes)",
            range.start,
            range.end,
            offense.cop,
            source.len()
        );
        if source[range.clone()] != *correction.replacement.as_bytes() {
            candidates.push((offense, correction));
        }
    }
    candidates.sort_by_key(|(offense, correction)| {
        (
            correction.range.start,
            correction.range.end,
            offense.cop,
            correction.replacement.as_str(),
            offense.message.as_str(),
        )
    });

    let mut selected: Vec<&Correction> = Vec::new();
    for (_, correction) in candidates {
        if let Some(last) = selected.last() {
            let a = &last.range;
            let b = &correction.range;
            if b.start < a.end || (b.start == a.end && (a.is_empty() || b.is_empty())) {
                continue;
            }
        }
        selected.push(correction);
    }
    let mut bytes = Vec::new();
    let mut offset = 0;
    for correction in &selected {
        bytes.extend_from_slice(&source[offset..correction.range.start]);
        bytes.extend_from_slice(correction.replacement.as_bytes());
        offset = correction.range.end;
    }
    bytes.extend_from_slice(&source[offset..]);
    Ok(AppliedCorrections {
        bytes,
        count: selected.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offense::{Correction, Offense, Severity};

    fn offense(cop: &'static str, range: std::ops::Range<usize>, text: &str) -> Offense {
        Offense {
            cop,
            severity: Severity::Convention,
            message: "test correction".into(),
            line: 1,
            end_line: 1,
            col: 1,
            correction: Some(Correction {
                range,
                replacement: text.into(),
            }),
        }
    }

    #[test]
    fn applies_length_changing_edits_against_original_byte_offsets() {
        let offenses = [
            offense("Style/B", 4..5, "longer"),
            offense("Style/A", 0..2, ""),
            offense("Style/C", 6..6, "!"),
        ];
        let applied = apply(b"abcdef", &offenses).unwrap();
        assert_eq!(applied.bytes, b"cdlongerf!");
        assert_eq!(applied.count, 3);
    }

    #[test]
    fn resolves_overlaps_by_range_then_cop_independent_of_input_order() {
        let mut offenses = vec![
            offense("Style/Z", 1..4, "z"),
            offense("Style/A", 1..4, "a"),
            offense("Style/B", 2..3, "b"),
            offense("Style/C", 4..5, "c"),
        ];
        let applied = apply(b"123456", &offenses).unwrap();
        offenses.reverse();
        let reversed = apply(b"123456", &offenses).unwrap();
        assert_eq!(applied.bytes, b"1ac6");
        assert_eq!(applied.count, 2);
        assert_eq!(reversed.bytes, applied.bytes);
    }

    #[test]
    fn insertions_at_same_offset_or_replacement_boundary_conflict() {
        let offenses = [
            offense("Style/A", 0..0, "a"),
            offense("Style/B", 0..0, "b"),
            offense("Style/C", 0..1, "c"),
            offense("Style/D", 1..1, "d"),
        ];
        let applied = apply(b"x", &offenses).unwrap();
        assert_eq!(applied.bytes, b"axd");
        assert_eq!(applied.count, 2);
    }

    #[test]
    fn preserves_raw_bytes_crlf_and_unicode() {
        let offenses = [offense("Style/A", 3..5, "ö")];
        let applied = apply(b"\xff\r\n\xc3\xa9\r\n", &offenses).unwrap();
        assert_eq!(applied.bytes, b"\xff\r\n\xc3\xb6\r\n");
    }

    #[test]
    fn ignores_nonfixable_and_noop_offenses() {
        let mut no_fix = offense("Style/A", 0..1, "x");
        no_fix.correction = None;
        let applied = apply(b"x", &[no_fix, offense("Style/B", 0..1, "x")]).unwrap();
        assert_eq!(applied.bytes, b"x");
        assert_eq!(applied.count, 0);
    }

    #[test]
    fn rejects_invalid_ranges_without_panicking() {
        for range in [std::ops::Range { start: 2, end: 1 }, 0..4, 4..4] {
            assert!(apply(b"abc", &[offense("Style/A", range, "x")]).is_err());
        }
    }
}
