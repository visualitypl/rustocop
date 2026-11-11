//! Unix permission policy comes from Ruby3.4 file.c rb_stat_x, not access(2).
use super::*;

#[cfg(unix)]
#[test]
fn execute_permission_uses_root_owner_group_other_precedence() {
    for mode in 0..=0o777 {
        for (uid, owner, group, mask) in [
            (0, 123, false, 0o111),
            (1000, 1000, true, 0o100),
            (1000, 1000, false, 0o100),
            (1000, 2000, true, 0o010),
            (1000, 2000, false, 0o001),
        ] {
            assert_eq!(
                executable(mode, owner, uid, group),
                mode & mask != 0,
                "mode={mode:o}, uid={uid}, owner={owner}, group={group}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn from_open_file_is_not_a_path_lookup_or_directory_permission_check() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.rb");
    std::fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let input = std::fs::File::open(&path).unwrap();
    let expected = input.metadata().unwrap();
    std::fs::remove_file(&path).unwrap();
    let metadata = SuppliedInputMetadata::from_file(&input).unwrap();
    assert_eq!(metadata.identity.inode, expected.ino());
    assert!(!metadata.executable);
    assert!(SuppliedInputMetadata::from_file(&std::fs::File::open(dir.path()).unwrap()).is_none());
}

#[cfg(unix)]
#[test]
fn native_ruby34_stat_matches_all_execute_bit_combinations() {
    use std::os::unix::fs::PermissionsExt;
    let Some(ruby) = std::env::var_os("RUSTOCOP_RUBY34") else {
        eprintln!("SKIP native Ruby3.4 metadata check: set RUSTOCOP_RUBY34");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let modes = [
        0o644, 0o755, 0o744, 0o654, 0o645, 0o754, 0o745, 0o655, 0o7444,
    ];
    let mut paths = Vec::new();
    let mut expected = Vec::new();
    for mode in modes {
        let path = dir.path().join(format!("{mode:o}.rb"));
        std::fs::write(&path, b"#!/usr/bin/ruby\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        expected.push(
            SuppliedInputMetadata::from_file(&std::fs::File::open(&path).unwrap())
                .unwrap()
                .executable
                .to_string(),
        );
        paths.push(path);
    }
    // Authored metadata-only program: no fixture/source Ruby is executed.
    let output = std::process::Command::new(ruby).args(["-e", "abort RUBY_VERSION unless RUBY_VERSION.start_with?('3.4.'); ARGV.each { |p| puts File.stat(p).executable? }"])
        .args(paths).output().expect("native Ruby 3.4 must run");
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", expected.join("\n"))
    );
}

#[cfg(not(unix))]
#[test]
fn unsupported_platform_metadata_is_inapplicable() {
    let file = tempfile::NamedTempFile::new().unwrap();
    assert!(SuppliedInputMetadata::from_file(file.as_file()).is_none());
}
