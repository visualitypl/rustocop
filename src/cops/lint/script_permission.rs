//! `Lint/ScriptPermission` — read-only execute-permission checks for supplied files.
use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct ScriptPermission;

impl Cop for ScriptPermission {
    fn name(&self) -> &'static str {
        "Lint/ScriptPermission"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        // Only the snapshot from the opened regular disk input grants metadata
        // authority. Never stat a display path: stdin/virtual files (including
        // existing filenames), unavailable metadata and non-Unix are inapplicable.
        if !file.bytes.starts_with(b"#!")
            || file.supplied_input_metadata().is_none_or(|m| m.executable)
        {
            return;
        }
        let basename = file.path.file_name().unwrap_or_default().to_string_lossy();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line: 1,
            col: 1,
            end_line: 1,
            message: format!("Script file {basename} doesn't have execute permission."),
            // Upstream's only correction is chmod, not a source edit. The fixed policy
            // explicitly excludes it; keep the finding even under --fix.
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/script_permission_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Permission correction is an
    //! explicit non-source exception, not an omitted source fix.
    use super::*;
    use crate::{
        config::ResolvedConfig,
        pipeline::{lint_one, lint_str},
    };

    #[test]
    fn memory_sources_do_not_acquire_permission_authority_from_a_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("real.rb");
        std::fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
        for name in [&path, &dir.path().join("missing.rb")] {
            let report = lint_one(
                name,
                name,
                Some(b"#!/usr/bin/ruby\n"),
                &[&ScriptPermission],
                &ResolvedConfig::default(),
            );
            assert!(report.errors.is_empty());
            assert!(report.result.offenses.is_empty());
        }
        assert!(
            lint_str("#!/usr/bin/ruby\n")
                .iter()
                .all(|o| o.cop != "Lint/ScriptPermission")
        );
    }

    #[cfg(unix)]
    #[test]
    fn supplied_disk_modes_and_initial_shebang_boundaries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.rb");
        for mode in [0o644, 0o755] {
            for (source, shebang) in [
                ("#!/usr/bin/ruby", true),
                ("#!/usr/bin/ruby\r\n", true),
                ("#!", true),
                ("#! arbitrary interpreter\n", true),
                ("puts \"hello\"", false),
                ("", false),
                ("\n#!/usr/bin/ruby\n", false),
                (" #!/usr/bin/ruby\n", false),
                ("# ordinary\n#!/usr/bin/ruby\n", false),
                ("\u{feff}#!/usr/bin/ruby\n", false),
            ] {
                std::fs::write(&path, source).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
                let report = lint_one(
                    &path,
                    &path,
                    None,
                    &[&ScriptPermission],
                    &ResolvedConfig::default(),
                );
                assert!(report.errors.is_empty(), "{source:?}");
                assert_eq!(
                    report.result.offenses.len(),
                    usize::from(mode == 0o644 && shebang),
                    "{source:?}"
                );
                for o in report.result.offenses {
                    assert_eq!((o.line, o.col, o.end_line), (1, 1, 1));
                    assert_eq!(
                        o.message,
                        "Script file script.rb doesn't have execute permission."
                    );
                    assert_eq!(o.severity, crate::offense::Severity::Error);
                    assert!(o.correction.is_none());
                }
            }
        }
        assert!(!ScriptPermission.safe_fixes());
        assert!(ScriptPermission.default_enabled());
        assert!(ScriptPermission.path_policy().is_none());
    }

    #[cfg(not(unix))]
    #[test]
    fn non_unix_disk_inputs_are_inapplicable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.rb");
        std::fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
        let report = lint_one(
            &path,
            &path,
            None,
            &[&ScriptPermission],
            &ResolvedConfig::default(),
        );
        assert!(report.errors.is_empty());
        assert!(report.result.offenses.is_empty());
    }
}
