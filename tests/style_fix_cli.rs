//! new Style corrections share the atomic, one-snapshot CLI contract.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const ONLY: &str = "Style/WordArray,Style/NumericLiterals,Style/MapJoin,Layout/SpaceAfterComma";
const BEFORE: &[u8] = b"values = ['one','two']\nnumber = 12345\n['x'].map { |x| x.to_s }.join\n";
const AFTER: &[u8] = b"values = %w[one two]\nnumber = 12_345\n['x'].map { |x| x.to_s }.join\n";
fn project(config: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join("rustocop.toml"), config).unwrap();
    fs::write(d.path().join("x.rb"), BEFORE).unwrap();
    d
}
fn cli(root: &Path, args: &[&str], status: i32) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(status), "{args:?}: {o:?}");
    assert!(o.stderr.is_empty(), "{o:?}");
    o
}
fn summary(o: &Output, expected: &str) {
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).lines().last(),
        Some(expected),
        "{o:?}"
    );
}
#[test]
fn selectors_exclusions_and_disables_keep_exact_counts_bytes_and_exit_codes() {
    for (config, except, after, summary_text) in [
        (
            "",
            "Lint",
            AFTER,
            "1 file inspected, 2 corrections applied, 1 offense remaining",
        ),
        (
            "",
            "Style/WordArray",
            b"values = ['one', 'two']\nnumber = 12_345\n['x'].map { |x| x.to_s }.join\n".as_slice(),
            "1 file inspected, 2 corrections applied, 1 offense remaining",
        ),
        (
            "[cops.\"Style/WordArray\"]\nexclude = [\"x.rb\"]\n",
            "Lint",
            b"values = ['one', 'two']\nnumber = 12_345\n['x'].map { |x| x.to_s }.join\n".as_slice(),
            "1 file inspected, 2 corrections applied, 1 offense remaining",
        ),
        (
            "exclude = [\"x.rb\"]\n",
            "Lint",
            BEFORE,
            "0 files inspected, 0 corrections applied, 0 offenses remaining",
        ),
    ] {
        let d = project(config);
        let path = d.path().join("x.rb");
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        cli(
            d.path(),
            &["--only", ONLY, "--except", except, "x.rb"],
            if config.starts_with("exclude") { 0 } else { 1 },
        );
        assert_eq!(fs::read(&path).unwrap(), BEFORE);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
        let fixed = cli(
            d.path(),
            &["--fix", "--only", ONLY, "--except", except, "x.rb"],
            if config.starts_with("exclude") { 0 } else { 1 },
        );
        summary(&fixed, summary_text);
        assert_eq!(fs::read(&path).unwrap(), after);
    }
    // --only forces the disabled department; --except still wins.
    let d = project(
        "[cops.Style]\nenabled = false\n[cops.Layout]\nenabled = false\n[cops.Lint]\nenabled = false\n[cops.Naming]\nenabled = false\n[cops.Security]\nenabled = false\n",
    );
    cli(d.path(), &["--fix", "x.rb"], 0);
    assert_eq!(fs::read(d.path().join("x.rb")).unwrap(), BEFORE);
    let fixed = cli(d.path(), &["--fix", "--only", ONLY, "x.rb"], 1);
    summary(
        &fixed,
        "1 file inspected, 2 corrections applied, 1 offense remaining",
    );
    assert_eq!(fs::read(d.path().join("x.rb")).unwrap(), AFTER);
}
#[test]
fn disabled_spans_and_error_uncorrectable_and_clean_files_are_not_written() {
    for prefix in ["rustocop", "rubocop"] {
        let d = project("");
        let path = d.path().join("x.rb");
        let source = format!(
            "# {prefix}:disable Style, Layout\n{}",
            String::from_utf8_lossy(BEFORE)
        );
        fs::write(&path, &source).unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        let fixed = cli(d.path(), &["--fix", "--only", ONLY, "x.rb"], 0);
        summary(
            &fixed,
            "1 file inspected, 0 corrections applied, 0 offenses remaining",
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
    let d = project("");
    fs::write(d.path().join("broken.rb"), b"def broken(\n").unwrap();
    fs::write(d.path().join("clean.rb"), b"123\n").unwrap();
    fs::write(
        d.path().join("uncorrectable.rb"),
        b"['x'].map { |x| x.to_s }.join\n",
    )
    .unwrap();
    let paths = ["broken.rb", "clean.rb", "uncorrectable.rb"];
    let before: Vec<_> = paths
        .iter()
        .map(|p| {
            (
                fs::read(d.path().join(p)).unwrap(),
                fs::metadata(d.path().join(p)).unwrap().modified().unwrap(),
            )
        })
        .collect();
    let out = cli(d.path(), &["--fix", "--only", ONLY, "."], 2);
    summary(
        &out,
        "4 files inspected, 2 corrections applied, 2 offenses remaining, 3 errors",
    );
    assert_eq!(fs::read(d.path().join("x.rb")).unwrap(), AFTER);
    for (name, (bytes, time)) in paths.iter().zip(before) {
        assert_eq!(fs::read(d.path().join(name)).unwrap(), bytes);
        assert_eq!(
            fs::metadata(d.path().join(name))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
    }
}
#[cfg(unix)]
#[test]
fn expanded_style_fixes_atomically_replace_preserve_permissions_and_leave_no_tempfiles() {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    };
    let d = project("");
    let path = d.path().join("x.rb");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    symlink("x.rb", d.path().join("alias.rb")).unwrap();
    let mut original = fs::File::open(&path).unwrap();
    let inode = original.metadata().unwrap().ino();
    let fixed = cli(d.path(), &["--fix", "--only", ONLY, "x.rb", "alias.rb"], 1);
    summary(
        &fixed,
        "2 files inspected, 2 corrections applied, 2 offenses remaining",
    );
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert!(
        fs::symlink_metadata(d.path().join("alias.rb"))
            .unwrap()
            .is_symlink()
    );
    let mut bytes = Vec::new();
    original.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, BEFORE);
    assert_eq!(fs::read(&path).unwrap(), AFTER);
    let metadata = fs::metadata(&path).unwrap();
    let stable = cli(d.path(), &["--fix", "--only", ONLY, "x.rb"], 1);
    summary(
        &stable,
        "1 file inspected, 0 corrections applied, 1 offense remaining",
    );
    assert_eq!(fs::metadata(&path).unwrap().ino(), metadata.ino());
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        metadata.modified().unwrap()
    );
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 4);
}
