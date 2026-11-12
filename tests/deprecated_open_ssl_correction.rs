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
#[test]
fn closed_candidate_loses_eligibility_under_uncertain_contexts_without_losing_findings() {
    let dir = project();
    for suffix in [
        "p __LINE__",
        "p __ENCODING__",
        "File.read(__FILE__)",
        "DATA.read\n__END__\npayload",
        "RubyVM::InstructionSequence.of(method(:puts))",
        "ObjectSpace.each_object {}",
        "GC.stat",
        "TracePoint.new(:call) {}.enable",
        "p binding",
        "eval('1')",
        "at_exit { puts 'later' }",
    ] {
        unchanged(
            dir.path(),
            format!("{SAFE}{suffix}\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
    for source in [
        SAFE.replace("require 'openssl'", "require 'other'"),
        SAFE.replace("require 'openssl'", "require('openssl')"),
        SAFE.replace("require 'openssl'", "require 'openssl'; require 'other'"),
        SAFE.replace("[OpenSSL", "BEGIN { puts 'before' }; [OpenSSL"),
        SAFE.replace("[OpenSSL", "END { puts 'after' }; [OpenSSL"),
        SAFE.replace("[OpenSSL", "module M; [OpenSSL") + "end\n",
        SAFE.replace("[OpenSSL", "x = [OpenSSL"),
        SAFE.replace("[OpenSSL", "if true; [OpenSSL") + "end\n",
        SAFE.replace("::SHA256.digest(", "::SHA256::digest("),
        SAFE.replace("::SHA256.digest(", "::SHA256.digest( # retained\n"),
        SAFE.replace("'foo'", "'é'"),
        SAFE.replace("'foo'", "%q(foo)"),
        SAFE.replace("'foo'", "'f\\oo'"),
        SAFE.replace("'foo'", "\"#{1}\""),
        SAFE.replace("'foo'", "'foo', &proc {}"),
        SAFE.replace("'foo')]", "'foo') {}]"),
        SAFE.replace("'foo')]", "'foo'), nil]"),
        "require 'openssl'\n[OpenSSL::Digest::SHA256.digest(<<~DOC)]\nfoo\nDOC\n".into(),
    ] {
        unchanged(
            dir.path(),
            source.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
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
fn native(ruby: &str, authored: &str) -> Output {
    Command::new(ruby)
        .args(["--parser=prism", "-w", "-e", authored])
        .output()
        .unwrap()
}
#[test]
fn native_actual_openssl_digest_values_bindings_encodings_warnings_and_errors() {
    if crate::support::runtime::skips("pins Ruby 3.4.1 and the versions of its bundled libraries") {
        return;
    }
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let out = native(
        &ruby,
        "require 'openssl'; abort 'wrong native version' unless RUBY_VERSION == '3.4.1'; p [OpenSSL::VERSION, OpenSSL::OPENSSL_LIBRARY_VERSION, OpenSSL::Digest::SHA256.superclass.name, OpenSSL::Digest.respond_to?(:digest)]",
    );
    assert!(
        out.status.success(),
        "actual native OpenSSL is REQUIRED, not nil/nil evidence: {out:?}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("[\"3.3.0\", \"OpenSSL 3.0.13 30 Jan 2024\", \"OpenSSL::Digest\", true]"),
        "{out:?}"
    );
    assert!(out.stderr.is_empty(), "{out:?}");
    let dir = project();
    for root in ["", "::"] {
        for literal in ["'foo'", "\"authored task proof\"", "''", "'a:b-c !'"] {
            for prefix in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
                "# coding: ISO-8859-1\n",
                "\u{feff}",
            ] {
                let before = format!(
                    "{prefix}require 'openssl'\n[{root}OpenSSL::Digest::SHA256.digest({literal})]\n"
                );
                fs::write(dir.path().join("a.rb"), &before).unwrap();
                let out = cli(dir.path(), &["--only", COP, "--fix", "a.rb"]);
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                let after = fs::read_to_string(dir.path().join("a.rb")).unwrap();
                assert_eq!(
                    after,
                    before.replace("::SHA256.digest(", ".digest('SHA256', ")
                );
                // Execute only these authored witnesses, never reference/corpus rows.
                let a = native(&ruby, &before);
                let b = native(&ruby, &after);
                assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
                assert_eq!(a.stdout, b.stdout);
                assert_eq!(a.stderr, b.stderr);
                assert!(a.stderr.is_empty(), "{a:?}");
                // Instrumented observation of the genuine returned value and bindings.
                let observe = |source: &str| {
                    // Keep magic encoding/frozen comments at the beginning.
                    let body = source.strip_prefix(prefix).unwrap();
                    format!(
                        "{prefix}p [(begin\n{body}end).map {{ |v| [v.class.name, v.encoding.name, v.bytes, v.unpack1('H*'), v.frozen?] }}, TOPLEVEL_BINDING.local_variables]\n"
                    )
                };
                let a = native(&ruby, &observe(&before));
                let b = native(&ruby, &observe(&after));
                assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
                assert_eq!(a.stdout, b.stdout, "{before}");
                assert_eq!(a.stderr, b.stderr);
                assert!(a.stderr.is_empty());
                let text = String::from_utf8_lossy(&a.stdout);
                if literal == "'foo'" {
                    assert!(
                        text.contains(
                            "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"
                        ),
                        "{text}"
                    );
                }
                assert!(
                    text.contains("\"String\", \"ASCII-8BIT\"") && text.ends_with(", []]\n"),
                    "{text}"
                );
            }
        }
    }
    // Actual errors are measured for held inputs; not an edit claim.
    for argument in ["nil", "1", "'foo', 'bar'", ""] {
        let a = native(
            &ruby,
            &format!(
                "require 'openssl'; begin; OpenSSL::Digest::SHA256.digest({argument}); rescue => e; p [e.class.name, e.message]; end"
            ),
        );
        let b = native(
            &ruby,
            &format!(
                "require 'openssl'; begin; OpenSSL::Digest.digest('SHA256'{}); rescue => e; p [e.class.name, e.message]; end",
                if argument.is_empty() {
                    String::new()
                } else {
                    format!(", {argument}")
                }
            ),
        );
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(a.stderr.is_empty() && b.stderr.is_empty());
        if matches!(argument, "nil" | "1") {
            assert_eq!(a.stdout, b.stdout);
        } else {
            assert_ne!(
                a.stdout, b.stdout,
                "arity error messages are observably different"
            );
        }
        unchanged(
            dir.path(),
            format!("require 'openssl'\n[OpenSSL::Digest::SHA256.digest({argument})]\n").as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
#[test]
fn native_real_constructor_dispatch_and_observer_counterexamples_stay_unedited() {
    let Ok(ruby) = crate::support::runtime::ruby() else {
        return;
    };
    let dir = project();
    for (before, after) in [
        (
            "p OpenSSL::Digest::SHA256.new.class.name",
            "p OpenSSL::Digest.new('SHA256').class.name",
        ),
        (
            "p OpenSSL::Cipher::AES.new(128, :GCM).class.name",
            "p OpenSSL::Cipher.new('aes-128-gcm').class.name",
        ),
        (
            "p OpenSSL::Cipher::Cipher.new('aes-128-gcm').class.name",
            "p OpenSSL::Cipher.new('aes-128-gcm').class.name",
        ),
        (
            "GC.disable; OpenSSL::Digest::SHA256.digest('foo'); p ObjectSpace.each_object(OpenSSL::Digest::SHA256).count",
            "GC.disable; OpenSSL::Digest.digest('SHA256', 'foo'); p ObjectSpace.each_object(OpenSSL::Digest::SHA256).count",
        ),
        (
            "class OpenSSL::Digest; def self.digest(*); 'custom'; end; end; p OpenSSL::Digest::SHA256.digest('foo')",
            "class OpenSSL::Digest; def self.digest(*); 'custom'; end; end; p OpenSSL::Digest.digest('SHA256', 'foo')",
        ),
        (
            "class OpenSSL::Digest::SHA256; def self.digest(*); 'custom'; end; end; p OpenSSL::Digest::SHA256.digest('foo')",
            "class OpenSSL::Digest::SHA256; def self.digest(*); 'custom'; end; end; p OpenSSL::Digest.digest('SHA256', 'foo')",
        ),
        (
            "p OpenSSL::Digest::SHA256.digest('foo').class; p RubyVM::InstructionSequence.compile('OpenSSL::Digest::SHA256.digest(\"foo\")').disasm",
            "p OpenSSL::Digest.digest('SHA256', 'foo').class; p RubyVM::InstructionSequence.compile('OpenSSL::Digest.digest(\"SHA256\", \"foo\")').disasm",
        ),
    ] {
        let before = format!("require 'openssl'\n{before}\n");
        let after = format!("require 'openssl'\n{after}\n");
        let a = native(&ruby, &before);
        let b = native(&ruby, &after);
        assert!(a.status.success() && b.status.success(), "{a:?}\n{b:?}");
        assert!(
            a.stdout != b.stdout || a.stderr != b.stderr,
            "lost real counterexample: {before}"
        );
        unchanged(
            dir.path(),
            before.as_bytes(),
            &["--only", COP, "--fix", "a.rb"],
            1,
        );
    }
}
