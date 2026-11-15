//! execute only these controlled synthetic main scripts. Host Ruby
//! VM output is not claimed to be fixed-3.4 Prism evidence (unit tests are).
use std::{fs, path::Path, process::Command};

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

fn cli(dir: &Path, cop: &str, fix: bool, code: i32) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustocop"));
    command.current_dir(dir).args(["--only", cop]);
    if fix {
        command.arg("--fix");
    }
    let out = command.arg("case.rb").output().unwrap();
    assert_eq!(out.status.code(), Some(code), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn operator_recovery_serializer_metadata_controls_fix_and_converge_on_host_and_native() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    let host = Command::new("ruby").arg("--version").output().is_ok();
    for runtime in host
        .then(|| "ruby".to_owned())
        .into_iter()
        .chain(crate::support::runtime::ruby())
    {
        for observer in [
            "p Marshal.name",
            "p ::Marshal.class",
            "p Object::Marshal.object_id.class",
            "p defined?(Marshal)",
            "p Marshal::MAJOR_VERSION",
            "p Marshal::MINOR_VERSION",
            "p (Other ||= 42)",
            "p '(Marshal ||= nil).load(payload)'",
            "p 'Marshal.load(payload)'",
            "# Marshal.restore(payload)\np 1",
        ] {
            // Object IDs themselves are process-specific; only their stable
            // class is observed in the correcting runtime control.
            let source = format!("x=1; {observer}\n");
            let run = || {
                let out = Command::new(&runtime)
                    .arg("--disable-gems")
                    .arg(&path)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{runtime}: {out:?}");
                assert!(out.stderr.is_empty(), "{out:?}");
                out.stdout
            };
            fs::write(&path, &source).unwrap();
            let before = run();
            cli(dir.path(), "Layout/SpaceAroundOperators", true, 0);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                source.replacen("x=1", "x = 1", 1)
            );
            assert_eq!(run(), before);
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            cli(dir.path(), "Layout/SpaceAroundOperators", true, 0);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        }
    }
}

#[test]
fn explicit_fixed_34_prism_columns_and_ast_shape_are_separate_from_host_vm_output() {
    let probe = Command::new("ruby")
        .args(["-rprism", "-e", "Prism.parse('nil', version: '3.4')"])
        .output();
    if !probe.is_ok_and(|out| out.status.success()) {
        eprintln!("skipping explicit fixed-3.4 parser evidence: Ruby/Prism unavailable");
        return;
    }
    let script = r#"
      def nodes(source)
        parsed = Prism.parse(source, version: '3.4')
        abort parsed.errors.inspect unless parsed.success?
        all = []
        visit = ->(n) { all << n; n.compact_child_nodes.each { |child| visit.call(child) } }
        visit.call(parsed.value)
        all
      end
      before = nodes('x=1; p RubyVM::AbstractSyntaxTree.of(-> {}).first_column')
      after = nodes('x = 1; p RubyVM::AbstractSyntaxTree.of(-> {}).first_column')
      first = before.find { |n| n.is_a?(Prism::LambdaNode) }.location.start_column
      last = after.find { |n| n.is_a?(Prism::LambdaNode) }.location.start_column
      abort 'column evidence' unless last - first == 2
      before = nodes('def f; lambda { 1 }; end')
      after = nodes('def f; -> { 1 }; end')
      abort 'call shape' unless before.any? { |n| n.is_a?(Prism::CallNode) && n.name == :lambda }
      abort 'arrow shape' unless after.any? { |n| n.is_a?(Prism::LambdaNode) }
      abort 'distinct shapes' if before.any? { |n| n.is_a?(Prism::LambdaNode) }
      puts 'fixed-3.4 column and shape evidence'
    "#;
    let out = Command::new("ruby")
        .args(["-rprism", "-e", script])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(out.stdout, b"fixed-3.4 column and shape evidence\n");
}

#[test]
fn namespace_only_metadata_keeps_runtime_equivalent_canonical_fixes() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for observer in [
        "p RubyVM.name",
        "p RubyVM::AbstractSyntaxTree.name",
        "p RubyVM::InstructionSequence.class",
        "p defined?(RubyVM::InstructionSequence)",
        "p Object.const_get(:String).name",
        "p Object.method(:const_get).call(:String).name",
        "p eval('42', binding)",
        "p Object.class_eval('nil')",
        "p Object.class_eval { 1 }",
        "p Object.module_eval { 1 }",
        "p Object.instance_eval { 1 }",
        "p Object.send(:class_eval) { 1 }",
        "Object.class_eval { def unrelated; 1; end; alias other unrelated }; p Object.new.other",
        "Object.module_eval { def unrelated; 1; end; alias_method :other, :unrelated }; p Object.new.other",
    ] {
        let source = format!("x=1; {observer}\n");
        fs::write(&path, &source).unwrap();
        let before = ruby(&path);
        cli(dir.path(), "Layout/SpaceAroundOperators", true, 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source.replacen("x=1", "x = 1", 1)
        );
        assert_eq!(ruby(&path), before);
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        cli(dir.path(), "Layout/SpaceAroundOperators", true, 0);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
