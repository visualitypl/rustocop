//! `Lint/EmptyFile` — genuinely empty source, with fixed AllowComments=true.
use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct EmptyFile;

impl Cop for EmptyFile {
    fn name(&self) -> &'static str {
        "Lint/EmptyFile"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        // This lifecycle runs even when the main parse has no statements.
        // Whitespace, a BOM, comments and DATA are nonempty upstream too.
        if file.bytes.is_empty() {
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line: 1,
                col: 1,
                end_line: 1,
                message: "Empty file detected.".into(),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/empty_file_spec.rb, including the global
    //! offense outside expect_offense. MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::pipeline::lint_str;

    #[test]
    fn empty_source_reports_a_global_offense() {
        let found: Vec<_> = lint_str("")
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyFile")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        assert_eq!(found[0].message, "Empty file detected.");
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn fixed_allow_comments_does_not_treat_missing_ast_as_empty_source() {
        for source in [
            "# comment\n",
            "foo.bar\n",
            " ",
            "\n",
            "\r\n",
            "\t",
            "\u{feff}",
            "__END__\nraw\n",
        ] {
            assert!(
                lint_str(source).iter().all(|o| o.cop != "Lint/EmptyFile"),
                "{source:?}"
            );
        }
    }
}
