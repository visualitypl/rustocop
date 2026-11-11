//! `Layout/InitialIndentation` — the first token that is not a `#` comment
//! starts its line.
use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct InitialIndentation;

const BOM: &[u8] = b"\xef\xbb\xbf";

impl Cop for InitialIndentation {
    fn name(&self) -> &'static str {
        "Layout/InitialIndentation"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(token) = first_token(result, file) else {
            return;
        };
        // `range_with_surrounding_space(side: :left, newlines: false)`: the
        // spaces and tabs right before the token. None (a BOM, a vertical tab
        // or the line start) means no offense.
        let bytes = &file.bytes;
        let blanks = bytes[..token]
            .iter()
            .rev()
            .take_while(|&&b| matches!(b, b' ' | b'\t'))
            .count();
        if blanks == 0 {
            return;
        }
        let (line, mut col) = file.line_col(token);
        // The parser gem's column counts a line-1 byte order mark.
        if line == 1 && bytes.starts_with(BOM) {
            col += 1;
        }
        let correction = (!file
            .class_method_bindings(&result.node())
            .has_listed_source_observer(result.data_loc().is_some()))
        .then(|| Correction {
            range: token - blanks..token,
            replacement: String::new(),
        });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: "Indentation of first line in file detected.".into(),
            line,
            end_line: line,
            col,
        });
    }
}

/// `processed_source.tokens.find { |t| !t.text.start_with?('#') }`: skip
/// whitespace, escaped newlines and `#` comments (a `=begin` block comment is
/// a token). `None` for a file holding only those.
fn first_token(result: &ruby_prism::ParseResult<'_>, file: &SourceFile) -> Option<usize> {
    let bytes = &file.bytes;
    let comments: Vec<_> = result
        .comments()
        .map(|c| c.location().start_offset()..c.location().end_offset())
        .collect();
    let end = result
        .data_loc()
        .map_or(bytes.len(), |data| data.start_offset());
    let mut at = if bytes.starts_with(BOM) { BOM.len() } else { 0 };
    while at < end {
        match bytes[at] {
            b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' => at += 1,
            b'\\' if bytes[at + 1..].starts_with(b"\n") => at += 2,
            b'\\' if bytes[at + 1..].starts_with(b"\r\n") => at += 3,
            b'#' => {
                let i = comments.partition_point(|c| c.start < at);
                let comment = comments.get(i).filter(|c| c.start == at)?;
                at = comment.end;
            }
            _ => return Some(at),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/initial_indentation_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Findings,
    //! messages and single-pass corrected bytes match the fixed replay recorded
    //! in the pinned RuboCop reference run.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::Severity,
        pipeline::lint_str,
    };

    const MSG: &str = "Indentation of first line in file detected.";

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/InitialIndentation")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    #[test]
    fn pinned_examples() {
        for source in ["def f\nend\n", "", "# comment\nx = 1\n", "\u{feff}puts 1"] {
            assert_eq!(positions(source), vec![], "{source:?}");
        }
        for (source, at, after) in [
            ("  def f\n  end\n", (1, 3), "def f\n  end\n"),
            ("   # comment\n   x = 1\n", (2, 4), "   # comment\nx = 1\n"),
            ("\u{feff}  puts 1\n", (1, 4), "\u{feff}puts 1\n"),
            (
                "\u{feff}# comment\n  puts 1\n",
                (2, 3),
                "\u{feff}# comment\nputs 1\n",
            ),
        ] {
            assert_eq!(
                positions(source),
                vec![(at.0, at.1, MSG.into())],
                "{source:?}"
            );
            assert_correction(&InitialIndentation, source, after);
        }
    }

    #[test]
    fn metadata_is_fixed() {
        let cop = InitialIndentation;
        assert!(cop.default_enabled() && cop.safe_fixes());
        assert_eq!(cop.severity(), Severity::Convention);
        assert_eq!(cop.ruby_versions(), Default::default());
    }

    #[test]
    fn tokens_comments_and_lookalikes() {
        for source in [
            "# only\n  # comments\n",
            "\n\n",
            "=begin\n  doc\n=end\n  x = 1\n",
            "x = 1\n  y = 2\n",
            "\x0bx = 1\n",
            "__END__\n  data\n",
            "  # c\n__END__\n  x\n",
        ] {
            assert_eq!(positions(source), vec![], "{source:?}");
        }
        for (source, at, after) in [
            ("\n\n\t x = 1\n", (3, 3), "\n\nx = 1\n"),
            ("  # a\n\n  ;x = 1\n", (3, 3), "  # a\n\n;x = 1\n"),
            ("\\\n  x = 1\n", (2, 3), "\\\nx = 1\n"),
            ("\r\n  x = 1\r\n", (2, 3), "\r\nx = 1\r\n"),
            ("\x0b x = 1\n", (1, 3), "\x0bx = 1\n"),
        ] {
            assert_eq!(
                positions(source),
                vec![(at.0, at.1, MSG.into())],
                "{source:?}"
            );
            assert_correction(&InitialIndentation, source, after);
        }
    }

    #[test]
    fn whitespace_before_a_heredoc_opener_is_held_by_the_boundary_index() {
        // The shared heredoc boundary index keeps whitespace before `<<`
        // (it can separate a command argument); the finding stays.
        let source = "  <<~A\n  a\nA\n";
        assert_eq!(positions(source), vec![(1, 3, MSG.into())]);
        assert!(
            correction_offenses(&InitialIndentation, source)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn listed_source_observers_hold_the_edit() {
        for observer in [
            "p __FILE__",
            "p $0",
            "p caller_locations",
            "p SCRIPT_LINES__",
        ] {
            let source = format!("  x = 1\n{observer}\n");
            let found = correction_offenses(&InitialIndentation, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        let data = "  p DATA.read\n__END__\nx\n";
        assert!(
            correction_offenses(&InitialIndentation, data)[0]
                .correction
                .is_none()
        );
    }
}
