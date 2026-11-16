//! authored closed-source/native proofs, never corpus execution.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const COP: &str = "Lint/DeprecatedOpenSSLConstant";
const SAFE: &str = "require 'openssl'\n[OpenSSL::Digest::SHA256.digest('foo')]\n";
const AFTER: &str = "require 'openssl'\n[OpenSSL::Digest.digest('SHA256', 'foo')]\n";
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    dir
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn unchanged(root: &Path, source: &[u8], args: &[&str], code: i32) {
    let path = root.join("a.rb");
    fs::write(&path, source).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(root, args);
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), source);
    let new = fs::metadata(&path).unwrap();
    assert_eq!(
        (old.modified().unwrap(), old.permissions()),
        (new.modified().unwrap(), new.permissions())
    );
}
#[test]
fn actual_canonical_cli_envelopes_no_write_gates_and_idempotence() {
    let dir = project();
    for root in ["", "::"] {
        for literal in ["'foo'", "\"authored task proof\"", "''", "'a:b-c !'"] {
            for (prefix, eol) in [
                ("", "\n"),
                ("\u{feff}", "\n"),
                ("# frozen_string_literal: true\r\n", "\r\n"),
                ("# frozen_string_literal: false\n", "\n"),
                ("# coding: ISO-8859-1\r\n# preserve\r\n", "\r\n"),
            ] {
                let source = format!(
                    "{prefix}require \"openssl\"{eol}[{root}OpenSSL::Digest::SHA256.digest({literal})] # preserve{eol}"
                );
                let after = source.replace("::SHA256.digest(", ".digest('SHA256', ");
                unchanged(dir.path(), source.as_bytes(), &["--only", COP, "a.rb"], 1);
                let out = cli(dir.path(), &["--only", COP, "--fix", "a.rb"]);
                assert_eq!(out.status.code(), Some(0), "{source}\n{out:?}");
                assert_eq!(fs::read(dir.path().join("a.rb")).unwrap(), after.as_bytes());
                unchanged(
                    dir.path(),
                    after.as_bytes(),
                    &["--only", COP, "--fix", "a.rb"],
                    0,
                );
            }
        }
    }
    // Applicable candidates also obey the real config/directive/selector gates.
    for config in [
        "exclude = ['a.rb']\n".into(),
        format!("[cops.\"{COP}\"]\nexclude = ['a.rb']\n"),
    ] {
        fs::write(dir.path().join("rustocop.toml"), config).unwrap();
        unchanged(
            dir.path(),
            SAFE.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            0,
        );
    }
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    unchanged(
        dir.path(),
        SAFE.as_bytes(),
        &["--only", COP, "--except", COP, "--fix", "a.rb"],
        0,
    );
    for prefix in ["rustocop", "rubocop"] {
        for directive in [
            format!("# {prefix}:disable {COP}"),
            format!("# {prefix}:next -{COP}"),
            format!("# {prefix}:disable-next {COP}"),
            format!("# {prefix}:push -{COP}"),
        ] {
            let source = SAFE.replace("[OpenSSL", &format!("{directive}\n[OpenSSL"));
            unchanged(
                dir.path(),
                source.as_bytes(),
                &["--only", COP, "--fix", "a.rb"],
                0,
            );
        }
    }
    // ASCII-compatible source encodings and non-UTF8 comments stay byte-exact.
    for prefix in [
        b"# coding: ISO-8859-1\r\n# \xe9\r\n".as_slice(),
        b"# coding: Shift_JIS\r\n# \x82\xa0\r\n",
    ] {
        let source = [prefix, SAFE.as_bytes()].concat();
        fs::write(dir.path().join("a.rb"), &source).unwrap();
        let out = cli(dir.path(), &["--only", COP, "--fix", "a.rb"]);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            fs::read(dir.path().join("a.rb")).unwrap(),
            [prefix, AFTER.as_bytes()].concat()
        );
    }
}
#[cfg(unix)]
#[test]
fn genuine_candidate_atomic_readonly_symlink_validation_and_peer_composition() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = project();
    let path = dir.path().join("a.rb");
    fs::write(&path, SAFE).unwrap();
    symlink("a.rb", dir.path().join("link.rb")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let old = fs::metadata(&path).unwrap();
    let out = cli(dir.path(), &["--only", COP, "--fix", "link.rb"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), SAFE.as_bytes());
    assert_eq!(fs::metadata(&path).unwrap().ino(), old.ino());
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        old.modified().unwrap()
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions(),
        old.permissions()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    unchanged(
        dir.path(),
        format!("{SAFE}(\n").as_bytes(),
        &["--only", COP, "--fix", "a.rb"],
        2,
    );
    fs::write(&path, SAFE).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let out = cli(dir.path(), &["--only", COP, "--fix", "link.rb", "a.rb"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read(&path).unwrap(), AFTER.as_bytes());
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        fs::symlink_metadata(dir.path().join("link.rb"))
            .unwrap()
            .is_symlink()
    );
    // Two disjoint real corrections share the existing single atomic transaction.
    fs::write(
        &path,
        SAFE.replace("'openssl'", "\"openssl\"")
            .replace("\n[", "  \n["),
    )
    .unwrap();
    let args = [
        "--only",
        "Lint/DeprecatedOpenSSLConstant,Layout/TrailingWhitespace",
        "--fix",
        "link.rb",
    ];
    let out = cli(dir.path(), &args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let expected = AFTER.replace("'openssl'", "\"openssl\"");
    assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
    unchanged(dir.path(), expected.as_bytes(), &args, 0);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
}
