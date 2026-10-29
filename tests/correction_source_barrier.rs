//! Controlled main-script fixtures only; never execute lint inputs or corpus code.
use std::{fs, path::Path, process::Command};

const CASES: &[(&str, &str, &str)] = &[
    ("Style/StringLiterals", "x = 'hi'\n", "x = \"hi\"\n"),
    ("Layout/TrailingWhitespace", "x = 1  \n", "x = 1\n"),
    (
        "Style/ClassMethods",
        "class A\n  def A.f; end\nend\n",
        "class A\n  def self.f; end\nend\n",
    ),
];

fn ruby(path: &Path) -> Vec<u8> {
    let output = Command::new("ruby")
        .arg("--disable-gems")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}

fn fix(dir: &Path, cop: &str, file: &str, code: i32) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rustocop"))
        .current_dir(dir)
        .args(["--only", cop, "--fix", file])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn raw_main_source_observations_retain_offenses_bytes_mtime_and_safe_siblings() {
    assert_raw_observers(&[
        "p File.binread(__FILE__)",
        "p File.binread($0)",
        "p File.binread($PROGRAM_NAME)",
        "p File.binread(Process.argv0)",
        "path = __FILE__; reader = File.method(:binread); p reader.call(path)",
        "alias $script $PROGRAM_NAME; p File.read($script)",
        "p IO.read(($0 ||= nil))",
        "p File.binread((Process.argv0 ||= nil))",
        "p [Process].map(&:argv0).map { |path| File.binread(path) }",
        "p File.binread(Process.public_send(\"arg\" + \"v0\"))",
        "class << Process; alias script argv0; end; p File.binread(Process.script)",
        "def marker; end; p File.binread(method(:marker).source_location.first)",
        "p File.binread(caller_locations(0).first.absolute_path)",
        "begin; raise; rescue => e; p File.binread(e.backtrace_locations.first.path); end",
        "File.open(__FILE__) { |file| p file.read }",
        "f = File.new(__FILE__); p f.each_line.to_a; f.close",
        "class << File; alias acquire new; end; f = File.acquire(__FILE__); p f.to_a; f.close",
        "f = File.public_send(:new, __FILE__); p f.to_a; f.close",
        "p File.binread(DATA.path)\n__END__\npayload",
        "require \"pathname\"; p Pathname.new(__FILE__).each_line.to_a",
    ]);
}

#[test]
fn nested_reflection_preserves_raw_source_and_no_write() {
    assert_raw_observers(&[
        "p File.binread(Process.public_send(:public_send, \"argv0\"))",
        "p File.binread(Process.public_send(:send, \"__send__\", :public_send, \"argv0\"))",
        "p File.public_send(:send, \"binread\", Process.argv0)",
        "p Process.method(:public_send).call(\"argv0\").then { |path| File.binread(path) }",
        "class << Process; alias_method :dispatch, :public_send; end; p File.binread(Process.dispatch(\"argv0\"))",
        "class << Process; alias dispatch public_send; end; p File.binread(Process.dispatch(\"argv0\"))",
        "dispatch = :public_send.to_proc; p File.binread(dispatch.call(Process, \"argv0\"))",
    ]);
}

#[test]
fn repositioned_data_preserves_raw_source_and_no_write() {
    assert_raw_observers(&[
        "DATA.rewind; p DATA.read\n__END__\npayload",
        "handle = DATA; handle.seek(0); p handle.read\n__END__\npayload",
        "DATA.read; DATA.sysseek(0); p DATA.read\n__END__\npayload",
        "DATA.pos = 0; p DATA.read\n__END__\npayload",
        "class << DATA; alias restart rewind; alias bytes read; end; DATA.restart; p DATA.bytes\n__END__\npayload",
        "DATA.public_send(:seek, 0); p DATA.read\n__END__\npayload",
        "p DATA.pread(DATA.stat.size, 0)\n__END__\npayload",
    ]);
}

#[test]
fn io_aliases_and_enumerable_reads_preserve_raw_source_and_no_write() {
    assert_raw_observers(&[
        "DATA.rewind; p DATA.each.to_a\n__END__\npayload",
        "DATA.rewind; p DATA.each_codepoint.to_a\n__END__\npayload",
        "DATA.rewind; p DATA.to_a\n__END__\npayload",
        "DATA.rewind; p DATA.entries\n__END__\npayload",
        "DATA.rewind; p DATA.map(&:itself)\n__END__\npayload",
        "DATA.rewind; bytes = []; while (c = DATA.getbyte); bytes << c; end; p bytes\n__END__\npayload",
        "DATA.rewind; chars = []; while (c = DATA.getc); chars << c; end; p chars\n__END__\npayload",
        "handle = DATA; class << handle; alias restart rewind; alias lines each; end; handle.restart; p handle.lines.to_a\n__END__\npayload",
        "class << DATA; alias_method :consume, :collect; end; DATA.rewind; p DATA.consume(&:itself)\n__END__\npayload",
        "DATA.rewind; p DATA.public_send(:public_send, 'each_codepoint').to_a\n__END__\npayload",
        "DATA.rewind; reader = DATA.method(:getbyte); p Array.new(24) { reader.call }\n__END__\npayload",
        "restart = DATA.method(:rewind); reader = :entries.to_proc; restart.call; p reader.call(DATA)\n__END__\npayload",
        "def later(handle); handle.seek(0); handle.flat_map { |line| line.bytes }; end; p later(DATA)\n__END__\npayload",
        "DATA.rewind; p DATA.lazy.map(&:itself).force\n__END__\npayload",
        "DATA.rewind; p DATA.enum_for(:each_entry).to_a\n__END__\npayload",
        "DATA.rewind; p DATA.reduce('') { |memo, line| memo + line }\n__END__\npayload",
    ]);
}

#[test]
fn unknown_io_constructor_acquisitions_preserve_raw_source_and_no_write() {
    assert_raw_observers(&[
        "F = File; p F.new(__FILE__).to_a",
        "def reader; File; end; p reader.new(__FILE__).entries",
        "F = File; make = F.method(:new); p make.call(__FILE__).to_a",
        "F = File; p F.public_send(:public_send, 'new', __FILE__).to_a",
        "F = File; make = :new.to_proc; p make.call(F, __FILE__).to_a",
        "F = File; class << F; alias acquire new; end; p F.acquire(__FILE__).to_a",
        "def consume(handle); p handle.to_a; end; F = File; consume(F.new(__FILE__))",
        "F = File; def acquire(path); F.new(path); end; p acquire(__FILE__).to_a",
        // An exemption by constructor spelling alone is not proof if the
        // nominal metadata class or its constructor was visibly replaced.
        "module M; Struct = File; p Struct.new(__FILE__).to_a; end",
        "module M; end; class M::Struct < File; end; module M; p Struct.new(__FILE__).to_a; end",
        "class << Struct; def new(path); File.open(path); end; end; p Struct.new(__FILE__).to_a",
    ]);
}

#[test]
fn argf_implicit_consumers_preserve_raw_source_and_no_write() {
    assert_raw_observers(&[
        "ARGV << __FILE__; p ARGF.to_a",
        "ARGV << __FILE__; p ::ARGF.each.to_a",
        "ARGV << __FILE__; bytes = []; while (c = ARGF.getbyte); bytes << c; end; p bytes",
        "ARGV << __FILE__; chars = []; while (c = $<.getc); chars << c; end; p chars",
        "ARGV << __FILE__; handle = Object::ARGF; p handle.map(&:itself)",
        "ARGV << __FILE__; p Object.const_get(:ARGF).entries",
        "ARGV << __FILE__; p ARGF.public_send(:public_send, 'to_a')",
        "ARGV << __FILE__; reader = ARGF.method(:each); p reader.call.to_a",
        "ARGV << __FILE__; reader = :to_a.to_proc; p reader.call(ARGF)",
        "ARGV << __FILE__; def consume(handle); handle.entries; end; p consume(ARGF)",
    ]);
}

#[test]
fn allocated_io_initializers_preserve_raw_source_and_no_write() {
    assert_raw_observers(&[
        "f = File.allocate; f.send(:initialize, __FILE__); p f.to_a",
        "f = File.allocate; f.__send__('initialize', __FILE__); p f.entries",
        "F = File; f = F.allocate; f.send(:send, :initialize, __FILE__); p f.to_a",
        "f = File.allocate; f.method(:initialize).call(__FILE__); p f.to_a",
        "f = File.allocate; init = f.method(\"initial\\x69ze\"); init.call(__FILE__); p f.to_a",
        "class << File; alias acquire allocate; end; f = File.acquire; f.send(:initialize, __FILE__); p f.to_a",
        "f = File.allocate; class << f; alias acquire initialize; public :acquire; end; f.acquire(__FILE__); p f.to_a",
        "def acquire(handle, path); handle.send(:initialize, path); end; f = File.allocate; acquire(f, __FILE__); p f.to_a",
    ]);
}

#[test]
fn tracepoint_paths_preserve_raw_source_and_no_write() {
    assert_raw_observers(&[
        "tp = TracePoint.new(:line) { |t| p File.binread(t.path); t.disable }; tp.enable\nx = 2",
        "Tracing = ::TracePoint; tp = Tracing.new(:line) { |t| p File.binread(t.public_send(\"path\")); t.disable }; tp.enable\nx = 2",
        "tp = Object::TracePoint.new(:line) { |t| getter = t.method(:path); p File.binread(getter.call); t.disable }; tp.enable\nx = 2",
    ]);
}

fn assert_raw_observers(observers: &[&str]) {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    let sibling = dir.path().join("safe.rb");
    for &(cop, prefix, canonical) in CASES {
        for observer in observers {
            for crlf in [false, true] {
                let encode = |s: String| {
                    if crlf {
                        format!("\u{feff}{}", s.replace('\n', "\r\n"))
                    } else {
                        s
                    }
                };
                let source = encode(format!("{prefix}{observer}\n"));
                fs::write(&path, &source).unwrap();
                let before = ruby(&path);
                fs::write(&path, encode(format!("{canonical}{observer}\n"))).unwrap();
                assert_ne!(ruby(&path), before, "{cop}: {source}");
                fs::write(&path, &source).unwrap();
                let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                // The sibling starts noncanonical and must converge independently.
                fs::write(&sibling, prefix).unwrap();
                for _ in 0..2 {
                    let report = fix(dir.path(), cop, ".", 1);
                    assert!(report.contains(cop), "{report}");
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        source.as_bytes(),
                        "{cop}: {source}"
                    );
                    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                }
                assert_eq!(fs::read_to_string(&sibling).unwrap(), canonical);
                assert_eq!(ruby(&path), before);
                fix(dir.path(), cop, "safe.rb", 0);
            }
        }
    }
}

#[test]
fn full_message_snippets_are_raw_observations_without_explicit_file_readers() {
    if Command::new("ruby").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for (cop, code, naive) in [
        (
            "Style/StringLiterals",
            "x = 'hi'; nil.foo",
            "x = \"hi\"; nil.foo",
        ),
        (
            "Layout/SpaceAroundOperators",
            "x = 1+2; nil.foo",
            "x = 1 + 2; nil.foo",
        ),
        (
            "Style/ClassMethods",
            "class A; def A.f; nil.foo; end; end; A.f",
            "class A; def self.f; nil.foo; end; end; A.f",
        ),
    ] {
        for observe in [
            "puts e.full_message(highlight: false)",
            "puts e.detailed_message(highlight: false)",
            "def selector; :full_message; end; puts [e].map(&selector)",
            "puts [e].map(&:full_message)",
            "puts e.public_send(:full_message)",
            "puts e.public_send(:public_send, \"full_message\", highlight: false)",
            "puts e.method(:send).call(\"full_message\", highlight: false)",
            "puts [e].map(&(\"full_\" + \"message\").to_sym)",
        ] {
            let encode = |code: &str| {
                format!("require \"error_highlight\"\nbegin\n{code}\nrescue => e\n{observe}\nend\n")
            };
            let source = encode(code);
            fs::write(&path, &source).unwrap();
            let before = ruby(&path);
            fs::write(&path, encode(naive)).unwrap();
            assert_ne!(ruby(&path), before, "{source}");
            fs::write(&path, &source).unwrap();
            let mtime = fs::metadata(&path).unwrap().modified().unwrap();
            for _ in 0..2 {
                let report = fix(dir.path(), cop, "case.rb", 1);
                assert!(report.contains(cop), "{report}");
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
                assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
            }
            assert_eq!(ruby(&path), before);
        }
    }
}

#[test]
fn payload_only_data_and_metadata_only_tracing_keep_layout_fixes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    let path = dir.path().join("case.rb");
    for observer in [
        "p ARGF.argv",
        "p ::ARGF.closed?",
        "handle = $<; p handle.argv",
        "p ARGF.filename",
        "ARGV << __FILE__; p ARGF.argv",
        "f = File.allocate; p f.class; p __FILE__",
        "p Object.allocate.class; p __FILE__",
        "p DATA.read\n__END__\npayload",
        "handle = DATA; p handle.gets\n__END__\npayload",
        "p DATA.each.to_a\n__END__\npayload",
        "p DATA.each_codepoint.to_a\n__END__\npayload",
        "p DATA.to_a\n__END__\npayload",
        "p DATA.map(&:itself)\n__END__\npayload",
        "bytes = []; while (c = DATA.getbyte); bytes << c; end; p bytes\n__END__\npayload",
        "chars = []; while (c = DATA.getc); chars << c; end; p chars\n__END__\npayload",
        "def consume(handle); handle.entries; end; p consume(DATA)\n__END__\npayload",
        "p DATA.lazy.map(&:itself).force\n__END__\npayload",
        "DATA.rewind; p :ok\n__END__\npayload",
        "DATA.rewind; p DATA.stat.file?\n__END__\npayload",
        "DATA.rewind; p DATA.closed?\n__END__\npayload",
        "tp = TracePoint.new(:line) { |t| p t.path; t.disable }; tp.enable\nx = 2",
        "tp = TracePoint.new(:line) { |t| p File.binread(\"/dev/null\"); t.disable }; tp.enable\nx = 2",
    ] {
        fs::write(&path, format!("x = 1  \n{observer}\n")).unwrap();
        let before = Command::new("ruby")
            .arg("--version")
            .output()
            .ok()
            .map(|_| ruby(&path));
        fix(dir.path(), "Layout/TrailingWhitespace", "case.rb", 0);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("x = 1\n{observer}\n")
        );
        if let Some(before) = before {
            assert_eq!(ruby(&path), before, "{observer}");
        }
        fix(dir.path(), "Layout/TrailingWhitespace", "case.rb", 0);
    }
}

#[test]
fn metadata_only_same_byte_line_preserving_string_fix_remains_available() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("rustocop.toml"), "").unwrap();
    for observer in [
        "p __FILE__",
        "p Process.public_send(:public_send, \"argv0\")",
        "p File.stat(__FILE__).size",
        "p Struct.new(:member).members; p __FILE__",
        "p Struct.public_send(:new, :member).members; p __FILE__",
        "p Class.new.name; p __FILE__",
        "p Module.new.name; p __FILE__",
        "require \"pathname\"; p Pathname.new(__FILE__).basename.to_s",
        "p Proc.new { __FILE__ }.call",
        "p [__FILE__].map(&:itself)",
        "p [:full_message, :detailed_message].map(&:to_s)",
        "def forward(&block); [1].map(&block); end; p forward { |n| n + 1 }; p $0",
        "def marker; end; p method(:marker).source_location",
        "p caller_locations(0).map(&:lineno)",
        "begin; raise; rescue => e; p e.backtrace; end",
    ] {
        fs::write(
            dir.path().join("case.rb"),
            format!("x = 'hi'\n{observer}\n"),
        )
        .unwrap();
        let before = Command::new("ruby")
            .arg("--version")
            .output()
            .ok()
            .map(|_| ruby(&dir.path().join("case.rb")));
        fix(dir.path(), "Style/StringLiterals", "case.rb", 0);
        if let Some(before) = before {
            assert_eq!(ruby(&dir.path().join("case.rb")), before);
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("case.rb")).unwrap(),
            format!("x = \"hi\"\n{observer}\n")
        );
    }
}
