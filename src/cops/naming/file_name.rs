//! `Naming/FileName` — fixed snake_case policy for the supplied basename.

use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct FileName;

impl Cop for FileName {
    fn name(&self) -> &'static str {
        "Naming/FileName"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let Some(basename) = file.path.file_name().or_else(|| {
            // Ruby File.basename('/') is '/', unlike Path::file_name. Path
            // equality uses components, so this also covers all-slash roots
            // without probing disk or changing dot/parent/empty handling.
            (file.path == std::path::Path::new("/")).then_some(std::ffi::OsStr::new("/"))
        }) else {
            return;
        };
        let basename = basename.to_string_lossy();
        // Fixed IgnoreExecutableScripts=true. A BOM, blank or comment before
        // the shebang does not qualify; permissions and disk state are irrelevant.
        if file.bytes.starts_with(b"#!") || allowed_basename(&file.path, &basename) {
            return;
        }
        // ExpectMatchingDefinition=false: no AST/namespace/acronym checks.
        let name = basename.strip_prefix('.').unwrap_or(&basename);
        // Ruby sub(/\.[^.]+$/, '') removes the first matching span. A
        // non-dot run can include LF, and $ can stop before an LF even when
        // another extension follows on a later line. Greedily use the last
        // such anchor before the next dot (or the end of the whole name).
        let extension = name.match_indices('.').find_map(|(start, _)| {
            let tail = &name[start + 1..];
            let end = match tail.find('.') {
                Some(next_dot) => tail[..next_dot].rfind('\n')?,
                None => tail.len(),
            };
            (end > 0).then_some(start..start + 1 + end)
        });
        let mut name = name.to_owned();
        if let Some(extension) = extension {
            name.replace_range(extension, "");
        }
        let name = name.replacen('+', "_", 1);
        // Ruby's ^/$ are line anchors, even for unusual filenames with LF.
        let good = name.split('\n').any(|line| {
            !line.is_empty()
                && line.chars().all(|c| {
                    c.is_ascii_digit()
                        || matches!(c, '_' | '.' | '?' | '!')
                        || crate::source::unicode_lowercase(c)
                })
        });
        if good {
            return;
        }
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            line: 1,
            col: 1,
            end_line: 1,
            message: format!("The name of this source file (`{basename}`) should use snake_case."),
        });
    }
}

/// Pinned AllCops Include patterns containing uppercase, plus the gemspec
/// exemption from Config#allowed_camel_case_file?. This is not discovery or a
/// public Include setting: explicit paths still reach this cop unchanged.
fn allowed_basename(path: &std::path::Path, name: &str) -> bool {
    if name
        .strip_suffix(".gemspec")
        // File.extname treats any run of leading dots as a hidden-name
        // prefix, not as an extension separator (e.g. `..gemspec`).
        .is_some_and(|stem| stem.chars().any(|c| c != '.'))
    {
        return true;
    }
    // Pinned Include globs do not traverse hidden directories. Ruby's hidden
    // filename fallback allows lib/.Fastfile, but not root .Fastfile (its
    // relative spelling contains no '/.'). Keep that exact path boundary.
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new(""));
    if parent.components().any(|part| matches!(part, std::path::Component::Normal(name) if name.to_string_lossy().starts_with('.')))
        || (parent.as_os_str().is_empty() && name.starts_with('.'))
    {
        return false;
    }
    name.ends_with("Fastfile")
        || matches!(
            name,
            "Appraisals"
                | "Berksfile"
                | "Brewfile"
                | "Buildfile"
                | "Capfile"
                | "Dangerfile"
                | "Deliverfile"
                | "Gemfile"
                | "Guardfile"
                | "Jarfile"
                | "Mavenfile"
                | "Podfile"
                | "Puppetfile"
                | "Rakefile"
                | "Schemafile"
                | "Snapfile"
                | "Steepfile"
                | "Thorfile"
                | "Vagrantfile"
        )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's naming/file_name_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{config::ResolvedConfig, cops::Registry, offense::Offense, pipeline::lint_one};
    use std::path::Path;

    fn offenses(path: &str, source: &str) -> Vec<Offense> {
        let registry = Registry::new();
        let active: Vec<_> = registry.cops().iter().map(|c| c.as_ref()).collect();
        let config = ResolvedConfig::resolve(Default::default(), None, &registry).unwrap();
        let report = lint_one(
            Path::new(path),
            Path::new(path),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        report
            .result
            .offenses
            .into_iter()
            .filter(|o| o.cop == "Naming/FileName")
            .collect()
    }

    fn assert_rows(rows: &[serde_json::Value]) {
        for row in rows {
            let found = offenses(
                row["file"].as_str().unwrap(),
                row["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let expected = row.get("rustocop_positions").unwrap_or(&row["positions"]);
            assert_eq!(serde_json::to_value(positions).unwrap(), *expected, "{row}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            let expected = row.get("rustocop_messages").unwrap_or(&row["messages"]);
            assert_eq!(serde_json::to_value(messages).unwrap(), *expected, "{row}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn all_expanded_pinned_calls_run_at_fixed_policy_with_supplied_paths() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns10_pinned.json")).unwrap();
        assert_eq!(rows.len(), 123);
        assert_eq!(
            rows.iter()
                .filter(|r| r["config_exclusion"].is_null())
                .count(),
            12
        );
        assert_rows(&rows);
    }

    #[test]
    fn all_source_documentation_examples_are_filenames_not_ruby_code() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns10_source.json")).unwrap();
        assert_eq!(rows.len(), 4);
        assert_rows(&rows);
    }

    #[test]
    fn path_boundaries_source_envelopes_unicode_and_normalization_match_pin() {
        let mut rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns10_paths.json")).unwrap();
        rows.extend(
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
                "../../../tests/fixtures/ns10_hidden_paths.json"
            ))
            .unwrap(),
        );
        assert_eq!(rows.len(), 107);
        assert_rows(&rows);
    }

    #[test]
    fn root_basename_and_relative_trailing_dot_and_parent_lookalikes() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns10_root_paths.json"))
                .unwrap();
        assert_eq!(rows.len(), 20);
        assert_rows(&rows);
        // Preserve the established non-root, missing-basename behavior. The
        // pin's config path helper errors when expanded file == cwd; these
        // are normalization controls, not claimed oracle matches.
        for path in ["", ".", "./", "lib/..", "lib/../", "lib/./.."] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path}");
        }
    }

    #[test]
    fn camel_case_and_extensionless_names_have_global_diagnostics() {
        for path in ["/some/dir/testCase.rb", "/some/dir/testCase", "a file.rb"] {
            let found = offenses(path, "print 1\n");
            assert_eq!(found.len(), 1, "{path}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "The name of this source file (`{}`) should use snake_case.",
                    Path::new(path).file_name().unwrap().to_str().unwrap()
                )
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn uppercase_include_exemptions_respect_hidden_path_boundaries() {
        for path in [
            ".Fastfile",
            ".hidden/Gemfile",
            "lib/.hidden/Gemfile",
            ".hidden/Fastfile",
            "lib/.hidden/.Fastfile",
        ] {
            assert_eq!(offenses(path, "print 1\n").len(), 1, "{path}");
        }
        for path in ["lib/.Fastfile", "/tmp/.Fastfile", ".hidden/BAD.gemspec"] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path}");
        }
    }

    #[test]
    fn pinned_exemptions_and_shebang_use_no_filesystem_metadata() {
        for path in ["test-case", "badName.rb"] {
            assert!(
                offenses(path, "#!/usr/bin/env ruby\nprint 1\n").is_empty(),
                "{path}"
            );
            assert_eq!(offenses(path, "# comment\n#!/usr/bin/env ruby\n").len(), 1);
        }
        for path in [
            "Rakefile.rb",
            "Gemfile",
            "lib/Rakefile",
            "Appraisals",
            "xFastfile",
            "bundler-console.gemspec",
            "BAD.gemspec",
        ] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path}");
        }
        for path in [
            "lib/Rakefile.rb",
            "Rakefile.rb.extra",
            "rakeFile.rb",
            "Gemfile.rb",
            "Gemfile.extra",
            "BAD.GEMSPEC",
            "FASTFILE",
        ] {
            assert_eq!(offenses(path, "print 1\n").len(), 1, "{path}");
        }
    }

    #[test]
    fn fixed_definition_policy_never_infers_namespaces_or_acronyms() {
        for source in [
            "",
            "print 1\n",
            "module Unrelated; class CLI; end; end\n",
            "HTTPServer = Struct.new\n",
        ] {
            assert!(
                offenses("lib/my/http_server.rb", source).is_empty(),
                "{source}"
            );
            assert_eq!(
                offenses("lib/my/HTTPServer.rb", source).len(),
                1,
                "{source}"
            );
        }
    }

    #[test]
    fn extension_substitution_uses_first_match_and_ruby_line_anchors() {
        for path in [
            "good.Upper\nBad.rb",
            "good.Upper\nBad",
            "Bad.Upper\ngood.rb",
            "Bad.\ngood.rb",
        ] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path:?}");
        }
        assert_eq!(offenses("Bad.x\ngood.Upper.rb", "print 1\n").len(), 1);
    }

    #[test]
    fn gemspec_exemption_requires_a_non_dot_stem() {
        for path in ["..gemspec", "lib/..gemspec"] {
            let found = offenses(path, "print 1\n");
            assert_eq!(found.len(), 1, "{path}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "The name of this source file (`{}`) should use snake_case.",
                    Path::new(path).file_name().unwrap().to_str().unwrap()
                )
            );
            assert!(found[0].correction.is_none());
        }
        for path in [".gemspec", "...gemspec", ".a.gemspec", "a..gemspec"] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path}");
        }
    }

    #[test]
    fn pinned_snake_case_lookalikes_are_accepted() {
        for path in [
            "test_case.rb",
            "test_case",
            "test_case.rake",
            "some_view.html.slim_spec.rb",
            "file?!.rb",
            "some_file.xlsx+mobile.axlsx",
            ".pryrc",
            "ünbound_sérvér.rb",
        ] {
            assert!(offenses(path, "print 1\n").is_empty(), "{path}");
        }
    }
}
