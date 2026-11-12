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
fn independent_column_shape_bytecode_and_retained_source_witnesses_preserve_runtime_and_disk() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for (cop, source, naive) in [
        (
            "Layout/SpaceAroundOperators",
            "x=1; p RubyVM::AbstractSyntaxTree.of(-> {}).first_column\n",
            "x = 1; p RubyVM::AbstractSyntaxTree.of(-> {}).first_column\n",
        ),
        (
            "Style/Lambda",
            "def f; lambda { 1 }; end; p RubyVM::AbstractSyntaxTree.of(method(:f)).children.last.type\n",
            "def f; -> { 1 }; end; p RubyVM::AbstractSyntaxTree.of(method(:f)).children.last.type\n",
        ),
        (
            "Style/Lambda",
            "def f; lambda { 1 }; end; p RubyVM::InstructionSequence.of(method(:f)).to_a.last.grep(Array).map(&:first)\n",
            "def f; -> { 1 }; end; p RubyVM::InstructionSequence.of(method(:f)).to_a.last.grep(Array).map(&:first)\n",
        ),
        (
            "Style/PerlBackrefs",
            "/a/ =~ 'a'; x = $&; p RubyVM.stat(:constant_cache_misses)\n",
            "/a/ =~ 'a'; x = Regexp.last_match(0); p RubyVM.stat(:constant_cache_misses)\n",
        ),
        (
            "Style/PerlBackrefs",
            "/a/ =~ 'a'; x = $&; p RubyVM.stat\n",
            "/a/ =~ 'a'; x = Regexp.last_match(0); p RubyVM.stat\n",
        ),
        (
            "Style/StringLiterals",
            "x = 'hi'\nRubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines\n",
            "x = \"hi\"\nRubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines\n",
        ),
    ] {
        for crlf in [false, true] {
            let encode = |s: &str| {
                if crlf {
                    format!("\u{feff}{}", s.replace('\n', "\r\n"))
                } else {
                    s.into()
                }
            };
            let source = encode(source);
            fs::write(&path, &source).unwrap();
            let before = ruby(&path);
            if source.contains("children.last.type") {
                assert_eq!(
                    before, b":ITER\n",
                    "shape witness must not depend on locations"
                );
            }
            fs::write(&path, encode(naive)).unwrap();
            assert_ne!(ruby(&path), before, "independent naive witness: {cop}");
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for fix in [false, true, true] {
                let report = cli(dir.path(), cop, fix, 1);
                assert!(report.contains(cop), "{report}");
                assert!(!report.contains("correction applied"), "{report}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                assert_eq!(ruby(&path), before);
            }
        }
    }
}

#[test]
fn aliased_reflective_deferred_and_unknown_selector_code_objects_preserve_runtime() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for observer in [
        "p ::RubyVM::AbstractSyntaxTree.of(-> {}).first_column",
        "AST = RubyVM::AbstractSyntaxTree; p AST.of(-> {}).first_column",
        "VM = RubyVM; p VM::AbstractSyntaxTree.of(-> {}).first_column",
        "def later; RubyVM::AbstractSyntaxTree.of(-> {}).first_column; end; p later",
        "getter = RubyVM::AbstractSyntaxTree.method(:of); p getter.call(-> {}).first_column",
        "p RubyVM::AbstractSyntaxTree.public_send(:send, :of, -> {}).first_column",
        "p Object.const_get(\"Ruby\" + \"VM\").const_get(\"Abstract\" + \"SyntaxTree\").of(-> {}).first_column",
        "p RubyVM.public_send(\"const\" + \"_get\", :AbstractSyntaxTree).of(-> {}).first_column",
        "p Module.instance_method(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.public_instance_method(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.public_send(:send, :instance_method, :const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.send(:public_send, :public_instance_method, :const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.method(:instance_method).call(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.method(:public_instance_method).call(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
    ] {
        let source = format!("x=1; {observer}\n");
        fs::write(&path, &source).unwrap();
        let before = ruby(&path);
        fs::write(&path, source.replacen("x=1", "x = 1", 1)).unwrap();
        assert_ne!(ruby(&path), before, "{source}");
        fs::write(&path, &source).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        for _ in 0..2 {
            let report = cli(dir.path(), "Layout/SpaceAroundOperators", true, 1);
            assert!(report.contains("Layout/SpaceAroundOperators"));
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(ruby(&path), before);
        }
    }
}

#[test]
fn mutated_core_getters_and_dynamic_evaluation_preserve_runtime_and_disk() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    // Controlled strings only: no user source or corpus is executed here.
    for (setup, getter) in [
        (
            "class << Object\n def const_get(name)\n  super(\"RubyVM::AbstractSyntaxTree\")\n end\nend",
            "Object.const_get(:String)",
        ),
        (
            "class << Object\n def const_get(name)\n  super(\"RubyVM::AbstractSyntaxTree\")\n end\nend",
            "Object.method(:const_get).call(:String)",
        ),
        (
            "class << Object\n def method(name)\n  super(:const_get)\n end\nend",
            "Object.method(:identity).call(\"RubyVM::AbstractSyntaxTree\")",
        ),
        (
            "class << Object\n def send(name)\n  super(:const_get, \"RubyVM::AbstractSyntaxTree\")\n end\nend",
            "Object.send(123)",
        ),
        (
            "def Object.class_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.class_eval { 1 }",
        ),
        (
            "def Object.module_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.module_eval { 1 }",
        ),
        (
            "def Object.instance_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.instance_eval { 1 }",
        ),
        (
            "def Object.class_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.send(:class_eval) { 1 }",
        ),
        (
            "def Object.module_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.public_send(:module_eval) { 1 }",
        ),
        (
            "def Object.instance_eval(*); super(\"RubyVM::AbstractSyntaxTree\", &nil); end",
            "Object.__send__(:instance_eval) { 1 }",
        ),
        ("K=eval(\"RubyVM::AbstractSyntaxTree\")", "K"),
        (
            "K=Kernel.public_send(:send, :eval, \"RubyVM::AbstractSyntaxTree\")",
            "K",
        ),
        ("K=method(:eval).call(\"RubyVM::AbstractSyntaxTree\")", "K"),
        (
            "alias acquire eval; K=acquire(\"RubyVM::AbstractSyntaxTree\")",
            "K",
        ),
        (
            "def acquire; eval(\"RubyVM::AbstractSyntaxTree\"); end; K=acquire",
            "K",
        ),
        ("K=binding.eval(\"RubyVM::AbstractSyntaxTree\")", "K"),
        ("K=Object.module_eval(\"RubyVM::AbstractSyntaxTree\")", "K"),
        (
            "K=:eval.to_proc.call(Kernel, \"RubyVM::AbstractSyntaxTree\")",
            "K",
        ),
    ] {
        for (cop, body, replacement) in [
            (
                "Layout/SpaceAroundOperators",
                format!("x=1; p {getter}.of(-> {{}}).first_column"),
                "x = 1",
            ),
            (
                "Style/Lambda",
                format!("def f; lambda {{ 1 }}; end; p {getter}.of(method(:f)).children.last.type"),
                "-> { 1 }",
            ),
        ] {
            let source = format!("{setup}\n{body}\n");
            fs::write(&path, &source).unwrap();
            let before = ruby(&path);
            let naive = if cop == "Style/Lambda" {
                source.replace("lambda { 1 }", replacement)
            } else {
                source.replace("x=1", replacement)
            };
            fs::write(&path, naive).unwrap();
            assert_ne!(ruby(&path), before, "{cop}: {source}");
            fs::write(&path, &source).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            for fix in [false, true, true] {
                let report = cli(dir.path(), cop, fix, 1);
                assert!(report.contains(cop), "{report}");
                assert!(!report.contains("correction applied"), "{report}");
                assert_eq!(fs::read(&path).unwrap(), source.as_bytes(), "{source}");
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                assert_eq!(ruby(&path), before);
            }
        }
    }
}

#[test]
fn operator_recovery_serialized_factories_preserve_host_and_native_runtime_and_disk() {
    if crate::support::runtime::skips(
        "pins compiled code or VM state that Ruby 4.0 builds differently",
    ) {
        return;
    }
    let mut runtimes = Vec::new();
    if Command::new("ruby").arg("--version").output().is_ok() {
        runtimes.push(("ruby".to_owned(), false));
    }
    if let Ok(native) = crate::support::runtime::ruby() {
        runtimes.push((native.clone(), false)); // Native default Prism: ISeq witnesses.
        runtimes.push((native, true)); // AST.of is unavailable for Prism-compiled ISeqs.
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for (runtime, parse_y) in runtimes {
        let native = runtime != "ruby";
        let run = || {
            let mut command = Command::new(&runtime);
            if parse_y {
                command.arg("--parser=parse.y");
            }
            let out = command.arg("--disable-gems").arg(&path).output().unwrap();
            assert!(
                out.status.success(),
                "{runtime}, parse.y={parse_y}: {out:?}"
            );
            assert!(out.stderr.is_empty(), "{out:?}");
            out.stdout
        };
        for (factory, cop, body, from, to) in [
            (
                "RubyVM::AbstractSyntaxTree",
                "Layout/SpaceAroundOperators",
                "x=1;p k.of(-> {}).first_column",
                "x=1",
                "x = 1",
            ),
            (
                "RubyVM::AbstractSyntaxTree",
                "Style/Lambda",
                "def f; lambda { 1 }; end; p k.of(method(:f)).children.last.type",
                "lambda { 1 }",
                "-> { 1 }",
            ),
            (
                "RubyVM::AbstractSyntaxTree",
                "Style/StringLiterals",
                "x = 'hi'; p k.parse_file(__FILE__, keep_script_lines: true).script_lines",
                "'hi'",
                "\"hi\"",
            ),
            (
                "RubyVM::InstructionSequence",
                "Layout/SpaceAroundOperators",
                "x=1;p k.of(-> {}).to_a[4][:code_location]",
                "x=1",
                "x = 1",
            ),
            (
                "RubyVM::InstructionSequence",
                "Style/Lambda",
                "def f; lambda { 1 }; end; p k.of(method(:f)).to_a.last.grep(Array).map(&:first)",
                "lambda { 1 }",
                "-> { 1 }",
            ),
        ] {
            if factory.ends_with("AbstractSyntaxTree")
                && body.contains("k.of(")
                && native
                && !parse_y
            {
                // Explicit API exclusion, not evidence from native default AST.of.
                continue;
            }
            // Fully hex-escaped, controlled module/class payloads. The AST name
            // is never a typed constant in the main parse or a substring proof.
            let mut bytes = vec![
                4,
                8,
                if factory.ends_with("AbstractSyntaxTree") {
                    b'm'
                } else {
                    b'c'
                },
                factory.len() as u8 + 5,
            ];
            bytes.extend_from_slice(factory.as_bytes());
            let payload = format!(
                "\"{}\"",
                bytes
                    .iter()
                    .map(|b| format!("\\x{b:02x}"))
                    .collect::<String>()
            );
            for setup in [
                format!("k=Marshal.load({payload})"),
                format!("k = (Marshal ||= nil).load({payload})"),
                format!("k = (::Marshal ||= nil).restore({payload})"),
                format!("k = (Object::Marshal ||= nil).public_send(:load, {payload})"),
                format!("getter = -> {{ (Marshal ||= nil) }}; k = getter.call.restore({payload})"),
                format!("k=::Marshal.restore({payload})"),
                format!("k=Object::Marshal.public_send(:send, :restore, {payload})"),
                format!("M=Marshal; k=M.load({payload})"),
                format!("getter=Marshal.method(:load); k=getter.call({payload})"),
                format!("getter=Marshal.public_method(:restore); k=getter.call({payload})"),
                format!(
                    "getter=Marshal.singleton_class.instance_method(:load); k=getter.bind_call(Marshal, {payload})"
                ),
                format!("k=:restore.to_proc.call(Marshal, {payload})"),
                format!("k=[{payload}].map(&Marshal.method(:load)).first"),
                format!("class << Marshal; alias acquire load; end; k=Marshal.acquire({payload})"),
                format!(
                    "class << Marshal; alias_method :acquire, :restore; end; k=Marshal.acquire({payload})"
                ),
                format!("def later(payload); Marshal.load(payload); end; k=later({payload})"),
                format!("selector=\"lo\"+\"ad\"; k=Marshal.public_send(selector, {payload})"),
            ] {
                for crlf in [false, true] {
                    let encode = |s: String| {
                        if crlf {
                            format!("\u{feff}{}", s.replace('\n', "\r\n"))
                        } else {
                            s
                        }
                    };
                    let source = encode(format!("{setup}\n{body}\n"));
                    fs::write(&path, &source).unwrap();
                    let before = run();
                    assert!(!before.is_empty());
                    if factory.ends_with("AbstractSyntaxTree") && cop == "Style/Lambda" {
                        assert_eq!(before, b":ITER\n"); // Independent location-free shape.
                    }
                    fs::write(&path, source.replacen(from, to, 1)).unwrap();
                    assert_ne!(run(), before, "{runtime}, parse.y={parse_y}: {source}");
                    fs::write(&path, &source).unwrap();
                    let modified = fs::metadata(&path).unwrap().modified().unwrap();
                    for fix in [false, true, true] {
                        let report = cli(dir.path(), cop, fix, 1);
                        assert!(report.contains(cop), "{report}");
                        assert!(!report.contains("correction applied"), "{report}");
                        assert_eq!(
                            fs::read(&path).unwrap(),
                            source.as_bytes(),
                            "{runtime}: {source}"
                        );
                        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
                        assert_eq!(run(), before);
                    }
                }
            }
        }
    }
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
