//! only controlled fixtures are executed, with the file as main script.
//! Host Ruby observations are separate from fixed-3.4 main-Prism unit tests.
use std::{fs, path::Path, process::Command};

const CASES: &[(&str, &str, &str)] = &[
    ("Style/Lambda", "f = lambda { 1 }\n", "f = -> { 1 }\n"),
    (
        "Style/ClassMethods",
        "class A\n  def A.f; end\nend\n",
        "class A\n  def self.f; end\nend\n",
    ),
    (
        "Style/ClassMethodsDefinitions",
        "class A\n  class << self\n    def f; end\n  end\nend\n",
        "class A\n  def self.f; end\nend\n",
    ),
    (
        "Style/EmptyLambdaParameter",
        "f = ->() { 1 }\n",
        "f = -> { 1 }\n",
    ),
    (
        "Style/SuperArguments",
        "def f(a); super(a); end\n",
        "def f(a); super; end\n",
    ),
    (
        "Style/KeywordParametersOrder",
        "def f(a: 1, b:); end\n",
        "def f(b:, a: 1); end\n",
    ),
    ("Style/TrailingCommaInArrayLiteral", "p [1,]\n", "p [1]\n"),
    (
        "Style/MethodCallWithArgsParentheses",
        "def f; puts  1; end\n",
        "def f; puts(1); end\n",
    ),
    ("Style/BarePercentLiterals", "p %Q(hello)\n", "p %(hello)\n"),
    (
        "Style/PercentLiteralDelimiters",
        "p %q[hello]\n",
        "p %q(hello)\n",
    ),
    ("Style/PercentQLiterals", "p %Q(hello)\n", "p %q(hello)\n"),
];

fn ruby(path: &Path) -> Vec<u8> {
    let out = Command::new("ruby")
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert!(!out.stdout.is_empty());
    out.stdout
}

fn cli(dir: &Path, args: &[&str], code: i32) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn proven_harmless_dispatch_and_forwarding_keep_canonical_runtime_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for &(cop, prefix, fixed) in CASES.iter().filter(|c| {
        matches!(
            c.0,
            "Style/ClassMethods" | "Style/TrailingCommaInArrayLiteral"
        )
    }) {
        for suffix in [
            "p [1].each(&nil).to_a",
            "p [1].map(&((:to_s)))",
            "p [1].map(&(\"to_s\".to_sym))",
            "p [1].map(&(\"to_s\".intern))",
            "p [1].map(&->(n) { n.to_s })",
            "def forward(&block); p [1].map(&((block))); end; forward { |n| n + 1 }",
            "def forward(&block); [1].each { |block| binding(&nil) }; p [1].map(&block); end; forward { |n| n + 1 }",
            "def unused; binding(&nil); end; def forward(&block); p [1].map(&block); end; forward { |n| n + 1 }",
            "def forward(&); p [1].map(&); end; forward { |n| n + 1 }",
            "def target; yield 1; end; def forward(...); p target(...); end; forward { |n| n + 1 }",
        ] {
            fs::write(&path, format!("{prefix}{suffix}\n")).unwrap();
            let before = ruby(&path);
            cli(dir.path(), &["--only", cop, "--fix", "case.rb"], 0);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!("{fixed}{suffix}\n")
            );
            assert_eq!(ruby(&path), before);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), &["--only", cop, "--fix", "case.rb"], 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn selectors_config_directives_exclusions_and_report_only_keep_bytes_and_mtime() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let config = dir.path().join("rustocop.toml");
    let path = dir.path().join("case.rb");
    let source = "f = lambda { 1 }\np File.binread(Process.argv0)\n";
    for (toml, prefix, args, code, reported) in [
        ("", "", vec!["--only", "Style/Lambda", "case.rb"], 1, true),
        (
            "",
            "",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            1,
            true,
        ),
        (
            "",
            "",
            vec![
                "--only",
                "Style/Lambda",
                "--except",
                "Style/Lambda",
                "--fix",
                "case.rb",
            ],
            0,
            false,
        ),
        (
            "[cops.\"Style/Lambda\"]\nenabled = false\n",
            "",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            1,
            true,
        ),
        (
            "[cops.\"Style\"]\nenabled = false\n",
            "",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            1,
            true,
        ),
        (
            "exclude = [\"case.rb\"]\n",
            "",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            0,
            false,
        ),
        (
            "[cops.\"Style/Lambda\"]\nexclude = [\"case.rb\"]\n",
            "",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            0,
            false,
        ),
        (
            "",
            "# rubocop:disable Style/Lambda\n",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            0,
            false,
        ),
        (
            "",
            "# rustocop:disable-next Style/Lambda\n",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            0,
            false,
        ),
        (
            "[cops.\"Style/Lambda\"]\nenabled = false\n",
            "# rustocop:enable Style/Lambda\n",
            vec!["--only", "Style/Lambda", "--fix", "case.rb"],
            1,
            true,
        ),
        (
            "[cops.\"Style\"]\nenabled = false\n[cops.\"Lint\"]\nenabled = false\n",
            "",
            vec!["--fix", "case.rb"],
            0,
            false,
        ),
        (
            "[cops.\"Style\"]\nenabled = false\n[cops.\"Lint\"]\nenabled = false\n[cops.\"Style/Lambda\"]\nenabled = true\n",
            "",
            vec!["--fix", "case.rb"],
            1,
            true,
        ),
        (
            "[cops.\"Style\"]\nenabled = false\n[cops.\"Lint\"]\nenabled = false\n",
            "# rustocop:enable Style/Lambda\n",
            vec!["--fix", "case.rb"],
            1,
            true,
        ),
    ] {
        fs::write(&config, toml).unwrap();
        let source = format!("{prefix}{source}");
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let report = cli(dir.path(), &args, code);
        assert_eq!(report.contains("Style/Lambda"), reported, "{report}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}

#[test]
fn guarded_file_does_not_block_canonical_sibling_file_convergence() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let guarded = "f = lambda { %Q[hello] }\np File.binread(Process.argv0)\n";
    let clean = "f = lambda { %Q[hello] }\n";
    fs::write(dir.path().join("guarded.rb"), guarded).unwrap();
    fs::write(dir.path().join("safe.rb"), clean).unwrap();
    let only = "Style/Lambda,Style/BarePercentLiterals,Style/PercentLiteralDelimiters,Style/PercentQLiterals";
    for _ in 0..4 {
        cli(dir.path(), &["--only", only, "--fix", "."], 1);
        assert_eq!(
            fs::read_to_string(dir.path().join("guarded.rb")).unwrap(),
            guarded
        );
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("safe.rb")).unwrap(),
        "f = -> { %(hello) }\n"
    );
    cli(dir.path(), &["--only", only, "--fix", "safe.rb"], 0);
}
