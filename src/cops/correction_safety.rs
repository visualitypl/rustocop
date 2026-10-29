//! Cross-family regression for the shared correction emission boundary.
mod code_objects;
mod heap;
mod positions;
mod style_emission;
use super::{Cop, NodeContext, Registry, run_cops};
use crate::{offense::Offense, source::SourceFile};

#[test]
fn source_node_and_leave_emissions_are_guarded_without_touching_other_files() {
    struct Emissions;
    fn emit(out: &mut Vec<Offense>, message: &str) {
        out.push(Offense {
            cop: "Style/TestEmission",
            severity: crate::offense::Severity::Convention,
            line: 1,
            col: 1,
            end_line: 1,
            message: message.into(),
            correction: Some(crate::offense::Correction {
                range: 0..1,
                replacement: "2".into(),
            }),
        });
    }
    impl super::Cop for Emissions {
        fn name(&self) -> &'static str {
            "Style/TestEmission"
        }
        fn check_source(&self, _: &SourceFile, out: &mut Vec<Offense>) {
            emit(out, "source");
        }
        fn wants_leave_node(&self) -> bool {
            true
        }
        fn check_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &super::NodeContext<'_, 'pr>,
            _: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                emit(out, "node");
            } else if node.as_call_node().is_some() {
                emit(out, "call");
            }
        }
        fn leave_node<'pr>(
            &self,
            node: &ruby_prism::Node<'pr>,
            _: &super::NodeContext<'_, 'pr>,
            _: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            if node.as_program_node().is_some() {
                emit(out, "leave");
            }
        }
    }
    for observer in [
        "p ObjectSpace.count_objects",
        "p ::Object::GC.stat",
        "p reader.stat",
        "((:autoload)).to_proc.call(Object, 'Trigger', 'helper.rb'); p File.stat('ignored')",
        "invoke(&((:autoload))); p File.stat('ignored')",
        "def later; ((:autoload)).to_proc; end; p File.stat('ignored')",
        "((:autoload)).send((:to_proc)); p File.stat('ignored')",
        "selector.to_proc; p File.stat('ignored')",
        "invoke(&selector); p File.stat('ignored')",
        "p File.binread(__FILE__)",
        "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines",
        "p retained.script_lines",
        "p $retained.source",
        "p $retained.public_send(:public_send, 'source')",
        "reader = $retained.method(:source); escape(reader)",
        "class << $retained; alias raw source; end; escape($retained)",
        "p retained.public_send(:public_send, 'script_lines')",
        "class << retained; alias lines script_lines; end; escape(retained)",
        "p RubyVM::AbstractSyntaxTree.of(-> {}).first_column",
        "p RubyVM::AbstractSyntaxTree.of(-> { x = 1 }).children",
        "p RubyVM::InstructionSequence.of(-> {}).to_a",
        "K = eval(code); p K.of(-> {}).first_column",
        "k = Marshal.load(payload); p k.of(-> {}).first_column",
        "k = ::Marshal.restore(payload); p k.of(-> {}).children",
        "k = (Marshal ||= nil).load(payload); p k.of(-> {}).first_column",
        "k = (Object::Marshal ||= nil).restore(payload); p k.of(-> {}).children",
        "def Object.class_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.class_eval { 1 }; p k.of(-> {}).first_column",
        "def Object.module_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.module_eval { 1 }; p k.of(-> {}).first_column",
        "def Object.instance_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.instance_eval { 1 }; p k.of(-> {}).first_column",
        "def Object.const_get(name); super('RubyVM::AbstractSyntaxTree'); end; p Object.const_get(:String).of(-> {}).first_column",
        "DATA.rewind; p DATA.each.to_a\n__END__\npayload",
        "DATA.rewind; p DATA.map(&:itself)\n__END__\npayload",
        "F = File; p F.new(__FILE__).to_a",
        "ARGV << __FILE__; p ARGF.to_a",
        "f = File.allocate; f.send(:initialize, __FILE__); p f.to_a",
    ] {
        let mut out = collect(&Emissions, "f\n");
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|o| o.correction.is_some()));
        let f = SourceFile::new("guarded.rb".into(), format!("1\n{observer}\n").into_bytes());
        let parsed = crate::parser::parse(&f.bytes);
        assert!(run_cops(&[&Emissions], &parsed, &f, &mut out).is_empty());
        assert!(out.len() >= 8);
        assert!(out[..4].iter().all(|o| o.correction.is_some()));
        assert!(
            out[4..].iter().all(|o| o.correction.is_none()),
            "{observer}"
        );
        assert_eq!(out[4].message, "source");
        assert_eq!(out[5].message, "node");
        assert!(out[6..out.len() - 1].iter().all(|o| o.message == "call"));
        assert_eq!(out.last().unwrap().message, "leave");
    }
}

fn collect(cop: &dyn super::Cop, source: &str) -> Vec<Offense> {
    let file = SourceFile::new("case.rb".into(), source.as_bytes().to_vec());
    let parsed = crate::parser::parse(&file.bytes);
    assert_eq!(parsed.errors().count(), 0, "{source}");
    file.record_comments(&parsed);
    file.record_continuations(&parsed.continuations);
    file.record_data_start(&parsed);
    file.record_warnings(&parsed);
    file.record_encoding(&parsed);
    let mut out = Vec::new();
    assert!(run_cops(&[cop], &parsed, &file, &mut out).is_empty());
    out
}

#[test]
fn finite_io_and_enumerable_surface_includes_alias_reflective_and_deferred_reads() {
    // Independent audit of Ruby 3.4 IO aliases and Enumerable's implicit each
    // entry points, not an executed corpus or an opaque detector baseline.
    let selectors = "getbyte getc each each_codepoint set_encoding_by_bom \
        all? any? chain chunk chunk_while collect collect_concat compact count \
        cycle detect drop drop_while each_cons each_entry each_slice each_with_index \
        each_with_object entries filter filter_map find find_all find_index first \
        flat_map grep grep_v group_by include? inject lazy map max max_by member? \
        min min_by minmax minmax_by none? one? partition reduce reject reverse_each \
        select slice_after slice_before slice_when sort sort_by sum take take_while \
        tally to_a to_h to_set uniq zip next next_values peek peek_values";
    let cop = super::layout::TrailingWhitespace;
    for name in selectors.split_whitespace() {
        for usage in [
            format!("DATA.{name}"),
            format!("DATA.public_send(:public_send, '{name}')"),
            format!("DATA.method(:{name}).call"),
            format!("reader = :{name}.to_proc; reader.call(DATA)"),
            format!("class << DATA; alias consume {name}; end; DATA.consume"),
            format!("class << DATA; alias_method :consume, '{name}'; end; DATA.consume"),
            format!("def later(h); h.{name}; end; later(DATA)"),
            format!("DATA.to_enum(:{name})"),
        ] {
            let source = format!("x = 1  \nDATA.rewind; {usage}\n__END__\npayload\n");
            let found = collect(&cop, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 6));
            assert_eq!(found[0].message, "Trailing whitespace detected.");
            assert!(found[0].correction.is_none(), "{source}");
            // ARGF can already start at the code prefix through ARGV; it
            // requires neither DATA nor repositioning. No Ruby is executed.
            let argf = source
                .replace("DATA.rewind; ", "ARGV << __FILE__; ")
                .replace("DATA", "ARGF");
            let found = collect(&cop, &argf);
            assert!(found[0].correction.is_none(), "{argf}");
            // The very same consumer without repositioning sees only payload.
            let payload_only = source.replace("DATA.rewind; ", "");
            let found = collect(&cop, &payload_only);
            assert!(found[0].correction.is_some(), "{payload_only}");
        }
    }
}

#[test]
fn argf_acquisitions_include_aliases_compound_and_indirect_lookup() {
    let cop = super::layout::TrailingWhitespace;
    // ARGV may already contain the script path at entry: no explicit __FILE__
    // or repositioning is needed to make an acquired ARGF stream uncertain.
    for acquisition in [
        "ARGF",
        "::ARGF",
        "Object::ARGF",
        "(ARGF ||= nil)",
        "(ARGF &&= other)",
        "(ARGF += other)",
        "$<",
        "($< ||= nil)",
        "($< &&= other)",
        "($< += other)",
        "(alias $input $<; $input)",
        "Object.const_get(:ARGF)",
        "Object.send(:const_get, 'ARGF')",
        "Object.method(:const_get).call(:ARGF)",
        "(lookup = :const_get.to_proc; lookup.call(Object, :ARGF))",
    ] {
        let source = format!("x = 1  \nhandle = {acquisition}; handle.to_a\n");
        let found = collect(&cop, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
        let metadata = source.replace("handle.to_a", "handle.closed?");
        let found = collect(&cop, &metadata);
        // A saved const_get proc is now an uncertain code-object getter:
        // unlike the immediate literal Method call, its later uses are not
        // connected by a general value-flow proof.
        assert_eq!(
            found[0].correction.is_some(),
            !acquisition.contains("lookup ="),
            "{metadata}"
        );
    }
}

#[test]
fn constructor_uncertainty_is_not_a_non_io_proof() {
    let cop = super::layout::TrailingWhitespace;
    for observer in [
        "F = File; escape(F.new(__FILE__))",
        "f = File.allocate; f.send(:initialize, __FILE__); escape(f)",
        "f = File.allocate; f.__send__('initialize', __FILE__); escape(f)",
        "f = File.allocate; f.send(:send, :initialize, __FILE__); escape(f)",
        "f = File.allocate; init = f.method(:initialize); escape(init); p __FILE__",
        "f = File.allocate; init = :initialize.to_proc; escape(init); p __FILE__",
        "f = File.allocate; class << f; alias acquire initialize; end; escape(f); p __FILE__",
        "f = File.allocate; f.initialize(__FILE__)",
        "F = File; ctor = F.method(:new); escape(ctor.call(__FILE__))",
        "F = File; escape(F.send(:public_send, :new, __FILE__))",
        "F = File; ctor = :new.to_proc; escape(ctor.call(F, __FILE__))",
        "F = File; escape([F].map(&:new)); p __FILE__",
        "F = File; class << F; alias ctor new; end; escape(F.ctor(__FILE__))",
        "p Struct.new(:member); p __FILE__; Struct = File",
        "p Struct.new(:member); p __FILE__; Object.const_set(:Struct, File)",
        "p Struct.new(:member); p __FILE__; Object::Struct = File",
        "p Struct.new(:member); p __FILE__; class << Struct; alias new acquire; end",
        "p Struct.new(:member); p __FILE__; Struct.singleton_class.alias_method(:new, :acquire)",
        "p Struct.new(:member); p __FILE__; Struct.singleton_class.public_send(:public_send, :alias_method, :new, :acquire)",
        "p Struct.new(:member); p __FILE__; Struct.public_send(:public_send, :define_singleton_method, :new) { acquire }",
        "p Struct.new(:member); p __FILE__; Object.public_send(:public_send, :const_set, :Struct, File)",
        "p Struct.new(:member); p __FILE__; Struct.method(:define_singleton_method).call(:new) { acquire }",
        "p Struct.new(:member); p __FILE__; Struct.define_singleton_method(:new) { acquire }",
        "p Struct.new(:member); p __FILE__; class Struct; end",
        "p Struct.new(:member); p __FILE__; module M; refine Struct do; end; end",
    ] {
        let source = format!("x = 1  \n{observer}\n");
        let found = collect(&cop, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    for observer in [
        "p Struct.new(:member).members; p __FILE__",
        "p Struct.send(:public_send, :new, :member).members; p __FILE__",
        "ctor = Struct.method(:new); p ctor.call(:member).members; p __FILE__",
        "p Class.new; p __FILE__",
        "p Module.new; p __FILE__",
        "p Object.new.class; p __FILE__",
        "p Struct.new(:member); p __FILE__; Struct.define_singleton_method(:other) { 1 }",
        "p Struct.new(:member); p __FILE__; Object.const_set(:Other, 1)",
        "p Proc.new { __FILE__ }.call",
        "tp = TracePoint.new(:line) { |t| p t.path }; p __FILE__",
        "p [__FILE__].map(&:itself)",
    ] {
        let source = format!("x = 1  \n{observer}\n");
        let found = collect(&cop, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_some(), "{source}");
    }
}

#[test]
fn every_registered_safe_family_uses_shared_observer_barriers() {
    // Reuse the independent detector evidence only as a source corpus, not as
    // a detector baseline. Each selected fixture must actually emit an edit.
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/style_evidence.json")).unwrap();
    let extra = [
        "foo\n# rubocop:enable Layout/LineLength\n",
        "# rubocop:disable Style/For # rubocop:disable Style/For\n",
        "a + b * c\n",
        "def +(arg); arg; end\n",
        "def foo(&callback); bar(&callback); end\n",
        "p <<~sql\nvalue\nsql\n",
        "begin; rescue => error; error; end\n",
        "begin; nil; rescue => Error; end\n",
        "NIL = nil; [::NIL]\n",
        "Fixnum = ::Integer; [::Fixnum]\n",
        "require 'uri'; [defined?(URI.regexp)]\n",
        "require 'openssl'; [OpenSSL::Digest::SHA256.digest('foo')]\n",
        "require 'bigdecimal'; def BigDecimal.new(value); ::Kernel.BigDecimal(value); end; [BigDecimal.new('1.25')]\n",
        "require 'erb'; [defined?(ERB.new('task-literal', nil, '-'))]\n",
        "a = 1  \n\n\n",
        "class A\n  end\n",
        "a = 1\n\n\nb = 2\n",
        "p ( 1 ); p({a:1}); foo {1}\n",
        "class A\n  def f\n    1\n    end\nend\n",
        "class A\n  attr :x\nend\n",
        "foo(**{a: 1}.merge(b: 2))\n",
        "def f; if block_given?; [1].each { |x| yield x }; end; end\n",
        "$VERBOSE = false\n$stderr.puts(\"hello\")\n",
        "module B; end\nmodule C; end\nclass A\n  include B, C\nend\n",
        "Dir.children(\".\").empty?\n",
        "File.open(\"file\").read\n",
        "a=1+2;puts([ 1 ,2 ],{ a: 1 }); a[ 0 ]\n",
        "foo { | a | a }\n",
        "def f(a =1);\n\tputs a\n end\n",
        "class A\n\n  def f\n\n    1\n\n  end\n\nend\n",
        "module A\n\n  def f\n    1\n  end\n  def g\n    2\n  end\n\nend\n",
    ];
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let config = crate::config::ResolvedConfig::resolve(
        crate::config::RawConfig::default(),
        Some(&dir.path().join("rustocop.toml")),
        &registry,
    )
    .unwrap();
    let path = dir.path().join("case.rb");
    let sibling = dir.path().join("safe.rb");
    let mut missing = Vec::new();
    let mut correcting_families = 0;
    for cop in registry.cops().iter().filter(|c| c.safe_fixes()) {
        let mut candidates: Vec<&str> = evidence
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["name"].as_str() == Some(cop.name()))
            .flat_map(|e| e["cases"].as_array().unwrap())
            .map(|c| c["source"].as_str().unwrap())
            .collect();
        candidates.extend(extra);
        let canonical = candidates.into_iter().find(|source| {
            collect(cop.as_ref(), source)
                .iter()
                .any(|o| o.correction.is_some())
        });
        let Some(source) = canonical else {
            if cop.name() == "Style/Copyright" {
                // Its fixed AutocorrectNotice is empty: no edit can be emitted.
                let found = collect(cop.as_ref(), "p File.binread(__FILE__)\n");
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none());
                continue;
            }
            if cop.name() == "Style/MissingElse" {
                // Fixed EmptyElse=both forbids both possible insertions;
                // upstream deliberately offers no correction at this setting.
                let found = collect(cop.as_ref(), "if ready; work; end\n");
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none());
                continue;
            }
            missing.push(cop.name());
            continue;
        };
        correcting_families += 1;
        let original = collect(cop.as_ref(), source);
        let applied = crate::corrections::apply(source.as_bytes(), &original).unwrap();
        assert_ne!(applied.bytes, source.as_bytes(), "{}", cop.name());
        for observer in [
            "p ObjectSpace.count_objects",
            "p ::GC.stat",
            "reader = :stat.to_proc; escape(reader)",
            "((:autoload)).to_proc; p File.stat('ignored')",
            "invoke(&((:autoload))); p File.stat('ignored')",
            "((:autoload)).send(:to_proc); p File.stat('ignored')",
            "selector.to_proc; p File.stat('ignored')",
            "p File.binread(__FILE__)",
            "p RubyVM::AbstractSyntaxTree.of(-> {}).first_column",
            "p RubyVM::AbstractSyntaxTree.of(-> { x = 1 }).children",
            "p RubyVM::InstructionSequence.of(-> {}).to_a",
            "K = eval(code); p K.of(-> {}).first_column",
            "k = Marshal.load(payload); p k.of(-> {}).first_column",
            "k = ::Marshal.restore(payload); p k.of(-> {}).children",
            "k = (Marshal ||= nil).load(payload); p k.of(-> {}).first_column",
            "k = (Object::Marshal ||= nil).restore(payload); p k.of(-> {}).children",
            "def Object.class_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.class_eval { 1 }; p k.of(-> {}).first_column",
            "def Object.module_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.module_eval { 1 }; p k.of(-> {}).first_column",
            "def Object.instance_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.instance_eval { 1 }; p k.of(-> {}).first_column",
            "def Object.const_get(name); super('RubyVM::AbstractSyntaxTree'); end; p Object.const_get(:String).of(-> {}).first_column",
            "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines",
            "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file_prism(__FILE__).script_lines",
            "p RubyVM::AbstractSyntaxTree.parse_file(__FILE__, keep_script_lines: true).script_lines",
            "I = RubyVM::InstructionSequence; escape(I.compile_file(__FILE__))",
            "p retained.script_lines",
            "p $retained.source",
            "p $retained.public_send(:public_send, 'source')",
            "reader = $retained.method(:source); escape(reader)",
            "class << $retained; alias raw source; end; escape($retained)",
            "def later(object); object.source; end",
            "p retained.public_send(:public_send, 'script_lines')",
            "reader = retained.method(:script_lines); escape(reader)",
            "class << retained; alias lines script_lines; end; escape(retained)",
            "def later(object); object.script_lines; end",
            "p File.mtime(__FILE__)",
            "p File.stat(__FILE__).ino",
            "p File.atime(__FILE__)",
            "FileUtils.mv(__FILE__, 'alias.rb'); p File.mtime('alias.rb'); FileUtils.mv('alias.rb', __FILE__)",
            "FileUtils.move(__FILE__, 'alias.rb'); p File.stat('alias.rb'); FileUtils.move('alias.rb', __FILE__)",
            "p File.stat(__FILE__)",
            "p File.stat(\"case\" + \".rb\")",
            "p test(?A, __FILE__)",
            "p File.binread($0)",
            "p File.binread($PROGRAM_NAME)",
            "p File.binread(Process.argv0)",
            "p File.binread((alias $script $0; $script))",
            "p File.binread(($0 ||= nil))",
            "p File.binread((Process.argv0 ||= nil))",
            "p File.binread(Process.public_send(\"arg\" + \"v0\"))",
            "path = __FILE__; p File.method(:binread).call(path)",
            "p File.binread(caller_locations(0).first.path)",
            "p error.full_message(highlight: false)",
            "p errors.map(&:full_message)",
            "p File.binread(Process.public_send(:public_send, \"argv0\"))",
            "p error.public_send(:public_send, \"full_message\")",
            "DATA.rewind; p DATA.read",
            "class << DATA; alias restart rewind; end; DATA.restart; p DATA.read",
            "DATA.rewind; p DATA.each.to_a",
            "DATA.rewind; p DATA.each_codepoint.to_a",
            "DATA.rewind; p DATA.to_a",
            "DATA.rewind; p DATA.map(&:itself)",
            "DATA.rewind; p DATA.getbyte",
            "DATA.rewind; p DATA.getc",
            "DATA.rewind; p DATA.lazy.flat_map(&:itself).force",
            "DATA.rewind; p DATA.public_send(:send, 'entries')",
            "class << DATA; alias consume each; end; DATA.rewind; p DATA.consume.to_a",
            "def later(handle); handle.seek(0); handle.entries; end; p later(DATA)",
            "F = File; p F.new(__FILE__).to_a",
            "ARGV << __FILE__; p ARGF.to_a",
            "ARGV << __FILE__; p $<.each.to_a",
            "f = File.allocate; f.send(:initialize, __FILE__); p f.to_a",
            "f = File.allocate; init = f.method(:initialize); escape(init); p __FILE__",
            "F = File; make = F.method(:new); escape(make.call(__FILE__))",
            "F = File; make = :new.to_proc; escape(make.call(F, __FILE__))",
            "tp = TracePoint.new(:line) { |t| p File.binread(t.path) }; tp.enable",
        ] {
            let eof = cop.name() == "Layout/TrailingEmptyLines";
            let guarded = if eof {
                format!("{observer}\n{source}")
            } else if let Some((code, data)) = source.split_once("__END__") {
                format!("{code}\n{observer}\n__END__{data}")
            } else {
                format!("{source}\n{observer}\n")
            };
            let found = collect(cop.as_ref(), &guarded);
            for old in &original {
                assert!(
                    found.iter().any(|o| (o.line, o.col, &o.message)
                        == (old.line + u32::from(eof), old.col, &old.message)),
                    "{}: {guarded}",
                    cop.name()
                );
            }
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{}: {guarded}",
                cop.name()
            );
            if matches!(
                observer,
                "p File.binread(__FILE__)"
                    | "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines"
                    | "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file_prism(__FILE__).script_lines"
                    | "p RubyVM::AbstractSyntaxTree.parse_file(__FILE__, keep_script_lines: true).script_lines"
                    | "p retained.script_lines"
                    | "p $retained.source"
                    | "class << $retained; alias raw source; end; escape($retained)"
                    | "class << retained; alias lines script_lines; end; escape(retained)"
                    | "p RubyVM::InstructionSequence.of(-> {}).to_a"
                    | "K = eval(code); p K.of(-> {}).first_column"
                    | "k = Marshal.load(payload); p k.of(-> {}).first_column"
                    | "k = ::Marshal.restore(payload); p k.of(-> {}).children"
                    | "k = (Marshal ||= nil).load(payload); p k.of(-> {}).first_column"
                    | "k = (Object::Marshal ||= nil).restore(payload); p k.of(-> {}).children"
                    | "FileUtils.mv(__FILE__, 'alias.rb'); p File.mtime('alias.rb'); FileUtils.mv('alias.rb', __FILE__)"
                    | "FileUtils.move(__FILE__, 'alias.rb'); p File.stat('alias.rb'); FileUtils.move('alias.rb', __FILE__)"
                    | "def Object.class_eval(*); super('RubyVM::AbstractSyntaxTree', &nil); end; k = Object.class_eval { 1 }; p k.of(-> {}).first_column"
                    | "def Object.const_get(name); super('RubyVM::AbstractSyntaxTree'); end; p Object.const_get(:String).of(-> {}).first_column"
            ) {
                // Exercise the real application path for every safe family;
                // these syntax fixtures are never executed as Ruby or commands.
                std::fs::write(&path, &guarded).unwrap();
                std::fs::write(&sibling, source).unwrap();
                let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
                let options = crate::pipeline::RunOptions {
                    only: vec![cop.name().into()],
                    fix: true,
                    ..Default::default()
                };
                let mut stable = false;
                for pass in 0..4 {
                    let before = std::fs::read(&sibling).unwrap();
                    let report = crate::pipeline::run(
                        &[path.clone(), sibling.clone()],
                        &options,
                        &config,
                        &registry,
                    )
                    .unwrap();
                    assert!(
                        report.errors.is_empty(),
                        "{}: {:?}",
                        cop.name(),
                        report.errors
                    );
                    assert!(report.internal_errors.is_empty());
                    assert_eq!(
                        std::fs::read(&path).unwrap(),
                        guarded.as_bytes(),
                        "{}",
                        cop.name()
                    );
                    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
                    assert!(
                        !report
                            .results
                            .iter()
                            .find(|r| r.path.ends_with("case.rb"))
                            .unwrap()
                            .offenses
                            .is_empty()
                    );
                    let after = std::fs::read(&sibling).unwrap();
                    if pass == 0 {
                        assert_eq!(after, applied.bytes, "{}", cop.name());
                    }
                    stable |= before == after;
                }
                assert!(stable, "{}: safe sibling did not converge", cop.name());
            }
        }
    }
    assert!(
        missing.is_empty(),
        "Need canonical fixtures or a specific no-edit reason: {missing:?}"
    );
    eprintln!(
        "Shared observer audit: {correcting_families} correcting safe families; Copyright/MissingElse fixed-policy no-edit exceptions."
    );
}
