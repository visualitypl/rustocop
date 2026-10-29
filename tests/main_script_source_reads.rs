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
fn main_script_runtime_source_data_and_line_observations_and_no_write() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for &(cop, prefix, naive) in CASES {
        for observer in [
            "$0",
            "$PROGRAM_NAME",
            "Process.argv0",
            "((::Process).argv0)",
            "(path = (($0)); path)",
            "(alias $script $PROGRAM_NAME; $script)",
            "(def path; $0; end; path)",
            "(reader = -> { Process.argv0 }; reader.call)",
            "(class << Process; alias script argv0; end; Process.script)",
            "($0 ||= nil)",
            "($PROGRAM_NAME &&= $PROGRAM_NAME)",
            "($0 += \"\")",
            "(Process.argv0 ||= nil)",
            "[Process].map(&((:argv0))).first",
            "((:argv0).to_proc).call(Process)",
            "Process.public_send(:argv0)",
            "Process.method(:argv0).call",
            "Process.send(\"argv0\")",
            "Process.public_send(\"arg\" + \"v0\")",
            "Process.method(\"argv0\").call",
            "eval(\"$0\")",
        ] {
            for crlf in [false, true] {
                let suffix = format!(
                    "p File.binread({observer})\np [__LINE__, DATA.pos, DATA.lineno, DATA.read]\n__END__\npayload\n"
                );
                let encode = |s: String| {
                    if crlf {
                        format!("\u{feff}{}", s.replace('\n', "\r\n"))
                    } else {
                        s
                    }
                };
                let source = encode(format!("{prefix}{suffix}"));
                let naive = encode(format!("{naive}{suffix}"));
                fs::write(&path, &source).unwrap();
                let before = ruby(&path);
                fs::write(&path, naive).unwrap();
                assert_ne!(ruby(&path), before, "{source}");
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                for _ in 0..2 {
                    let report = cli(dir.path(), &["--only", cop, "--fix", "case.rb"], 1);
                    assert!(report.contains(cop), "{report}");
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        source.as_bytes(),
                        "{cop}: {source}"
                    );
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
                assert_eq!(ruby(&path), before);
            }
        }
    }
}

fn assert_indirect_selector_no_write(observers: &[&str]) {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for &(cop, prefix, naive) in CASES {
        for observer in observers {
            for crlf in [false, true] {
                let encode = |prefix: &str| {
                    let source = format!("{prefix}{observer}\n");
                    if crlf {
                        format!("\u{feff}{}", source.replace('\n', "\r\n"))
                    } else {
                        source
                    }
                };
                let source = encode(prefix);
                fs::write(&path, &source).unwrap();
                let before = ruby(&path);
                fs::write(&path, encode(naive)).unwrap();
                assert_ne!(ruby(&path), before, "{source}");
                fs::write(&path, &source).unwrap();
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                for _ in 0..2 {
                    let report = cli(dir.path(), &["--only", cop, "--fix", "case.rb"], 1);
                    assert!(report.contains(cop), "{report}");
                    assert!(!report.contains("1 correction applied"), "{report}");
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        source.as_bytes(),
                        "{cop}: {source}"
                    );
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                }
                assert_eq!(ruby(&path), before);
            }
        }
    }
}

#[test]
fn dynamic_block_selectors_preserve_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "p [Process].map(&(\"arg\" + \"v0\").to_sym).map { |path| File.binread(path) }",
        "p [Process].map(&:\"arg#{'v0'}\").map { |path| File.binread(path) }",
        "selector = (\"arg\" + \"v0\").to_sym; p [Process].map(&selector).map { |path| File.binread(path) }",
        "selector = [\"arg\" + \"v0\"].map(&:to_sym).first; p [Process].map(&selector).map { |path| File.binread(path) }",
        "def selector; :\"arg#{'v0'}\"; end; p [Process].map(&selector).map { |path| File.binread(path) }",
    ]);
}

#[test]
fn symbol_table_selector_preserves_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "selector = Symbol.all_symbols.find { |s| s.to_s == \"arg\" + \"v0\" }; p [Process].map(&selector).map { |path| File.binread(path) }",
    ]);
}

#[test]
fn deserialized_selector_preserves_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "selector = Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); p [Process].map(&selector).map { |path| File.binread(path) }",
    ]);
}

#[test]
fn unknown_selector_origins_preserve_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "def selector; Symbol.all_symbols.find { |s| s.to_s == \"arg\" + \"v0\" }; end; p [Process].map(&selector).map { |path| File.binread(path) }",
        "@selector = Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); p [Process].map(&@selector).map { |path| File.binread(path) }",
        "SELECTOR = Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); p [Process].map(&SELECTOR).map { |path| File.binread(path) }",
        "def read(selector); p [Process].map(&selector).map { |path| File.binread(path) }; end; read(Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"))",
        "def read(&block); block = Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); p [Process].map(&block).map { |path| File.binread(path) }; end; read",
    ]);
}

#[test]
fn overridden_literal_conversions_preserve_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "class String; def to_sym; Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); end; end; p [Process].map(&(\"to_s\".to_sym)).map { |path| File.binread(path) }",
        "class String; def intern; Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); end; end; p [Process].map(&(\"to_s\".intern)).map { |path| File.binread(path) }",
        "class Symbol; def to_sym; Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); end; end; p [Process].map(&(:to_s.to_sym)).map { |path| File.binread(path) }",
        "class String; def selector; Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"); end; alias to_sym selector; end; p [Process].map(&(\"to_s\".to_sym)).map { |path| File.binread(path) }",
        "class String; define_method(:intern) { Marshal.load(\"\\x04\\b:\\narg\" + \"v0\") }; end; p [Process].map(&(\"to_s\".intern)).map { |path| File.binread(path) }",
    ]);
}

#[test]
fn binding_exposed_forwarding_preserves_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "def read(&block); binding.local_variable_set(:block, Marshal.load(\"\\x04\\b:\\narg\" + \"v0\")); p [Process].map(&block).map { |path| File.binread(path) }; end; read",
        "def read(&block); saved = binding; saved.local_variable_set(:block, Marshal.load(\"\\x04\\b:\\narg\" + \"v0\")); p [Process].map(&block).map { |path| File.binread(path) }; end; read",
        "def read(&block); [1].each { binding.local_variable_set(:block, Marshal.load(\"\\x04\\b:\\narg\" + \"v0\")) }; p [Process].map(&block).map { |path| File.binread(path) }; end; read",
    ]);
}

#[test]
fn block_bearing_and_aliased_binding_mutation_preserves_runtime_source_bytes_and_mtime() {
    let mutation = "local_variable_set(:block, Marshal.load(\"\\x04\\b:\\narg\" + \"v0\"))";
    let tail = "p [Process].map(&block).map { |path| File.binread(path) }; end; read";
    let observers: Vec<String> = [
        format!("def read(&block); binding(&nil).{mutation}; {tail}"),
        format!("def read(value = binding(&nil).{mutation}, &block); {tail}"),
        format!("def read(&block); binding(&-> {{}}).{mutation}; {tail}"),
        format!("def read(&block); binding(&block).{mutation}; {tail}"),
        format!("def read(&block); Kernel.binding(&nil).{mutation}; {tail}"),
        format!("def read(&block); saved = binding(&nil); saved.{mutation}; {tail}"),
        format!("def read(&block); callback = -> {{ binding(&nil).{mutation} }}; callback.call; {tail}"),
        format!("def read(&block); [1].each {{ binding(&nil).{mutation} }}; {tail}"),
        format!("def read(&block); send(:binding, &nil).{mutation}; {tail}"),
        format!("module Kernel; alias capture binding; end; def read(&block); capture.{mutation}; {tail}"),
        format!("Kernel.alias_method(:capture, :binding); def read(&block); capture.{mutation}; {tail}"),
        format!("def read(&block); name = 'bind' + 'ing'; send(name).{mutation}; {tail}"),
        format!("K = Kernel; def replace(b); b.{mutation}; end; def read(&block); replace(K.binding(&nil)); {tail}"),
        format!("K = Kernel; def replace(b); b.{mutation}; end; def read(receiver = K, &block); replace(receiver.binding(&nil)); {tail}"),
        format!("K = Kernel; def replace(b); b.{mutation}; end; def receiver; K; end; def read(&block); replace(receiver.binding(&nil)); {tail}"),
        format!("def replace(b); b.{mutation}; end; def read(&block); replace((self.binding ||= nil)); {tail}"),
    ].into();
    assert_indirect_selector_no_write(&observers.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn compound_proc_acquisition_preserves_runtime_source_bytes_and_mtime() {
    assert_indirect_selector_no_write(&[
        "selector = \"arg\" + \"v0\"; callback = (selector.to_sym.to_proc ||= nil); p File.binread(callback.call(Process))",
    ]);
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
