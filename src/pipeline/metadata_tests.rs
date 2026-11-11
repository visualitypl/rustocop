//! Disk-provenance seam and the real ScriptPermission snapshot consumer.
//! Shebang fixtures ported from RuboCop's script_permission_spec.rb
//! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).

use super::*;
use crate::cops::Cop;
use crate::source::SuppliedInputMetadata;
use std::sync::Mutex;

#[derive(Default)]
struct Probe(Mutex<Vec<(PathBuf, Option<SuppliedInputMetadata>)>>);
impl Cop for Probe {
    fn name(&self) -> &'static str {
        "Test/SuppliedInput"
    }
    fn check_source(&self, file: &SourceFile, _: &mut Vec<Offense>) {
        self.0
            .lock()
            .unwrap()
            .push((file.path.clone(), file.supplied_input_metadata().copied()));
    }
}

#[test]
fn memory_and_virtual_filenames_never_grant_metadata_io() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.rb");
    fs::write(&real, b"#!/usr/bin/ruby\n").unwrap();
    let probe = Probe::default();
    let config = ResolvedConfig::default();
    for path in [&real, &dir.path().join("missing.rb")] {
        let report = lint_one(
            path,
            Path::new("display.rb"),
            Some(b"#!/usr/bin/ruby\n"),
            &[&probe],
            &config,
        );
        assert!(report.errors.is_empty());
        let file =
            SourceFile::new(path.clone(), b"#!/usr/bin/ruby\n".to_vec()).with_source_path(path);
        assert!(file.supplied_input_metadata().is_none());
        // Loading provenance alone is not metadata authority either.
        assert!(
            file.with_read_path(path)
                .supplied_input_metadata()
                .is_none()
        );
    }
    assert_eq!(probe.0.lock().unwrap().len(), 2);
    assert!(probe.0.lock().unwrap().iter().all(|(_, m)| m.is_none()));
}

#[test]
fn unreadable_or_missing_disk_inputs_keep_existing_read_errors() {
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::default();
    for path in [dir.path().to_path_buf(), dir.path().join("missing.rb")] {
        let report = lint_one(
            &path,
            Path::new("reported.rb"),
            None,
            &[&probe],
            &ResolvedConfig::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].path, Path::new("reported.rb"));
        assert!(report.errors[0].message.starts_with("could not read file:"));
    }
    assert!(probe.0.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn actual_named_shebang_and_no_shebang_inputs_have_target_metadata() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("script.rb");
    let alias = dir.path().join("alias.rb");
    let hard = dir.path().join("hard.rb");
    for (mode, source) in [
        (0o644, "#!/usr/bin/ruby\n"),
        (0o755, "#!/usr/bin/ruby\n"),
        (0o755, "puts \"hello\"\n"),
        (0o755, ""),
    ] {
        fs::write(&target, source).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
        if !alias.exists() {
            symlink(&target, &alias).unwrap();
            fs::hard_link(&target, &hard).unwrap();
        }
        let before = fs::metadata(&target).unwrap();
        let probe = Probe::default();
        for path in [&target, &alias, &hard] {
            let report = lint_one(path, path, None, &[&probe], &ResolvedConfig::default());
            assert!(report.errors.is_empty());
        }
        let rows = probe.0.lock().unwrap();
        assert_eq!(rows.len(), 3);
        let metadata = rows[0].1.unwrap();
        assert_eq!(metadata.identity.device, before.dev());
        assert_eq!(metadata.identity.inode, before.ino());
        assert_eq!(metadata.executable, mode == 0o755);
        assert!(rows.iter().all(|(_, m)| *m == Some(metadata)));
        assert_eq!(rows[1].0, alias);
        let after = fs::metadata(&target).unwrap();
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert_eq!(after.mode(), before.mode());
        assert_eq!(after.ino(), before.ino());
        assert_eq!(fs::read(&target).unwrap(), source.as_bytes());
        assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn disk_snapshot_never_restats_a_replaced_or_missing_path() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.rb");
    fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let (bytes, metadata) = read_disk_input(&path).unwrap();
    assert!(!metadata.unwrap().executable);
    fs::remove_file(&path).unwrap();
    let probe = Probe::default();
    let config = ResolvedConfig::default();
    let selection = CopSelection::explicit(&[&probe], &config);
    for replaced in [false, true] {
        if replaced {
            fs::write(&path, b"# replacement\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let report = lint_one_in_mode(
            &path,
            Path::new("display.rb"),
            SourceInput::DiskSnapshot(&bytes, metadata),
            &selection,
            &config,
            CorrectionMode::None,
        );
        assert!(report.errors.is_empty());
        assert_eq!(probe.0.lock().unwrap().last().unwrap().1, metadata);
    }
    // Dangling links follow the existing read-error path, not a fabricated
    // non-executable metadata finding.
    let dangling = dir.path().join("dangling.rb");
    symlink(dir.path().join("missing.rb"), &dangling).unwrap();
    let report = lint_one(&dangling, &dangling, None, &[&probe], &config);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(probe.0.lock().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn script_permission_consumes_held_fd_snapshot_not_current_path_metadata() {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.rb");
    let display = Path::new("display.rb");
    let cop = crate::cops::lint::ScriptPermission;
    let config = ResolvedConfig::default();
    let cops: &[&dyn Cop] = &[&cop];
    let selection = CopSelection::explicit(cops, &config);
    for (original, replacement, expected) in [(0o644, 0o755, 1), (0o755, 0o644, 0)] {
        fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(original)).unwrap();
        let mut held = fs::File::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"# replacement\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(replacement)).unwrap();
        let metadata = SuppliedInputMetadata::from_file(&held);
        let mut bytes = Vec::new();
        held.read_to_end(&mut bytes).unwrap();
        for missing in [false, true] {
            if missing {
                fs::remove_file(&path).unwrap();
            }
            let report = lint_one_in_mode(
                &path,
                display,
                SourceInput::DiskSnapshot(&bytes, metadata),
                &selection,
                &config,
                CorrectionMode::None,
            );
            assert!(report.errors.is_empty());
            assert_eq!(report.result.offenses.len(), expected);
            if let Some(o) = report.result.offenses.first() {
                assert_eq!(
                    o.message,
                    "Script file display.rb doesn't have execute permission."
                );
                assert_eq!((o.line, o.col, o.end_line), (1, 1, 1));
                assert!(o.correction.is_none());
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn alias_fix_validation_reuses_original_permission_identity_snapshot() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    struct ObserveAndFix(std::sync::Arc<Probe>);
    impl Cop for ObserveAndFix {
        fn name(&self) -> &'static str {
            "Test/SuppliedInputFix"
        }
        fn safe_fixes(&self) -> bool {
            true
        }
        fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
            self.0.check_source(file, out);
            // Test-only canonical whitespace edit; ordinary shared guards/writer.
            if file.bytes == b"# before\n" {
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line: 1,
                    end_line: 1,
                    col: 1,
                    message: "test edit".into(),
                    correction: Some(crate::offense::Correction {
                        range: 2..8,
                        replacement: "after".into(),
                    }),
                });
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.rb");
    let alias = dir.path().join("alias.rb");
    fs::write(&path, b"# before\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    symlink(&path, &alias).unwrap();
    let original = fs::metadata(&path).unwrap();
    let probe = std::sync::Arc::new(Probe::default());
    let registry = Registry::from_cops(vec![Box::new(ObserveAndFix(probe.clone()))]);
    let report = run(
        &[path.clone(), alias.clone()],
        &RunOptions {
            fix: true,
            ..RunOptions::default()
        },
        &ResolvedConfig::default(),
        &registry,
    )
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.internal_errors.is_empty());
    assert_eq!(report.corrections_applied, 1);
    let rows = probe.0.lock().unwrap();
    assert_eq!(rows.len(), 4); // two aliases, original and validation passes
    let metadata = rows[0].1.unwrap();
    assert_eq!(metadata.identity.inode, original.ino());
    assert!(metadata.executable);
    assert!(rows.iter().all(|(_, m)| *m == Some(metadata)));
    assert_eq!(fs::read(&path).unwrap(), b"# after\n");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o751);
    assert_ne!(fs::metadata(&path).unwrap().ino(), original.ino());
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
}
