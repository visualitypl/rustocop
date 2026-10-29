//! Heap evidence is independent of the candidate's position and cop family.
use super::collect;
use crate::cops::{Cop, style::KeywordArgumentsMerging};

#[test]
fn actual_keyword_merges_keep_diagnostics_but_not_allocation_changing_edits() {
    let cop = KeywordArgumentsMerging;
    let candidate = "def f(**kw); end\nf(**{a: 1}.merge(b: 2))\n";
    let original = collect(&cop, candidate);
    assert_eq!(original.len(), 1);
    assert!(original[0].correction.is_some());
    for observer in [
        "p ObjectSpace.count_objects",
        "p ::ObjectSpace.count_objects_size",
        "p ::Object::GC.stat",
        "Space = ObjectSpace; p Space.count_objects",
        "def later; GC.stat; end; p later",
        "def later(reader); reader.stat; end; p later(acquire)",
        "p reader.count_objects",
        "p reader.stat_heap",
        "p reader.latest_gc_info",
        "p reader.total_allocated_objects",
        "p reader.malloc_allocations",
        "p reader.count_symbols",
        "p reader.dump_shapes",
        "p reader.latest_compact_info",
        "p reader.total_time",
        "reader.trace_object_allocations_debug_start",
        "reader.garbage_collect",
        "p reader.weakref_alive?",
        "reader.define_finalizer(value, callback)",
        "p reader.public_send(:public_send, 'stat')",
        "getter = reader.method(:count_objects); escape(getter)",
        "getter = :stat.to_proc; escape(getter)",
        "items.map(&:count_objects)",
        "class << reader; alias tally stat; end; p reader.tally",
        "reader.alias_method(:tally, :count_objects)",
        "p Object.const_get(name).stat",
        "p reader.public_send(name)",
    ] {
        // The reader can be after the candidate, before it, or deferred.
        for source in [
            format!("{candidate}{observer}\n"),
            format!("{observer}\n{candidate}"),
        ] {
            let found = collect(&cop, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].message, original[0].message);
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}

#[test]
fn metadata_exemption_requires_proved_dispatch_not_just_stat_spelling() {
    let cop = crate::cops::style::StringLiterals;
    for metadata in [
        "p File.stat('other.rb').file?",
        "p ::File.stat('other.rb').file?",
        "p Object::File.stat('other.rb').file?",
    ] {
        let source = format!("x = 'hi'\n{metadata}\n");
        assert!(collect(&cop, &source)[0].correction.is_some(), "{source}");
        for mutation in [
            "def File.stat(*); GC.stat; end",
            "def File.stat(*); external_reader; end",
            "class << File; alias stat acquire; end",
            "File.define_singleton_method(:stat) { acquire }",
            "Object.const_set(:File, acquire)",
            "File = acquire",
            "module M; refine File.singleton_class do; def stat(*); acquire; end; end; end; using M",
        ] {
            for source in [
                format!("{mutation}\n{source}"),
                format!("{source}{mutation}\n"),
            ] {
                assert!(collect(&cop, &source)[0].correction.is_none(), "{source}");
            }
        }
    }
    for metadata in [
        "p other.stat",
        "p Other::File.stat('other.rb')",
        "getter = File.method(:stat); escape(getter)",
    ] {
        let source = format!("x = 'hi'\n{metadata}\n");
        assert!(collect(&cop, &source)[0].correction.is_none(), "{source}");
    }
}

#[test]
fn immediate_core_namespace_metadata_is_non_heap_only_under_proved_dispatch() {
    let cop = crate::cops::style::StringLiterals;
    for observer in [
        "p GC.name",
        "p ::ObjectSpace.class",
        "p Object::GC.object_id",
        "p defined?(::GC)",
    ] {
        let source = format!("x = 'hi'\n{observer}\n");
        assert!(collect(&cop, &source)[0].correction.is_some(), "{source}");
        for mutation in [
            "def GC.name; acquire; end",
            "class << GC; alias name acquire; end",
            "Object.const_set(:GC, acquire)",
            "GC = acquire",
        ] {
            let source = format!("{source}{mutation}\n");
            assert!(collect(&cop, &source)[0].correction.is_none(), "{source}");
        }
    }
    // Mere acquisition, unknown receivers and unfamiliar namespaces are not
    // metadata proofs. The core module could escape and be consumed later.
    for observer in [
        "escape(GC)",
        "g = ObjectSpace; escape(g)",
        "p Other::GC.name",
        "p GC.method(:name).receiver",
        "p GC.name { effect }",
    ] {
        let source = format!("x = 'hi'\n{observer}\n");
        assert!(collect(&cop, &source)[0].correction.is_none(), "{source}");
    }
}

#[test]
fn symbol_proc_mutations_revoke_metadata_exemptions() {
    let cop = crate::cops::style::DirEmpty;
    for mutation in [
        ":autoload.to_proc.call(Object, 'Trigger', 'helper.rb')",
        "['Trigger', 'helper.rb'].then(&:autoload)",
        ":define_singleton_method.to_proc",
        "items.each(&:const_set)",
    ] {
        for metadata in ["p File.stat('ignored')", "p GC.name"] {
            let source = format!("a = Dir.children('.').empty?\n{mutation}\n{metadata}\n");
            let found = collect(&cop, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    // A non-mutating Symbol proc does not revoke a metadata proof.
    let source = "a = Dir.children('.').empty?\nitems.map(&:to_s)\np File.stat('ignored')\n";
    assert!(collect(&cop, source)[0].correction.is_some());
}

#[test]
fn wrapped_and_unresolved_proc_acquisitions_are_not_metadata_proofs() {
    let candidate = "a = Dir.children('.').empty?\n";
    for acquisition in [
        "((:autoload)).to_proc.call(Object, 'Trigger', 'helper.rb')",
        "invoke(&((:autoload)))",
        "def later; ((:autoload)).to_proc; end",
        "((:autoload)).public_send((:to_proc))",
        "((:autoload)).method((:to_proc))",
        "Object.public_send((:public_send), ((:autoload)), 'Trigger', 'helper.rb')",
        "selector.to_proc",
        "invoke(&selector)",
        "invoke(&(acquire()))",
        "(:auto.to_s + 'load').to_sym.to_proc",
        "((:send)).to_proc",
        "invoke(&((:public_send)))",
        "Object.method((:public_send))",
        "Object.send((selector))",
    ] {
        for metadata in ["p File.stat('ignored')", "p GC.name"] {
            let source = format!("{candidate}{acquisition}\n{metadata}\n");
            let found = collect(&crate::cops::style::DirEmpty, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}

#[test]
fn heap_fold_itself_handles_wrappers_reflection_and_unresolved_acquisition() {
    for acquisition in [
        "((:autoload)).to_proc",
        "invoke(&((:autoload)))",
        "def later; ((:autoload)).to_proc; end",
        "((:autoload)).send(:to_proc)",
        "((:autoload)).send((:to_proc))",
        "((:autoload)).method((:to_proc))",
        "Object.send((:send), ((:autoload)), 'Trigger', 'helper.rb')",
        "selector.to_proc",
        "invoke(&factory())",
        "invoke(&(\"autoload\".to_sym))",
        "invoke(&(\"auto\" + \"load\").to_sym)",
        "((:public_send)).to_proc",
        "Object.method((:send))",
    ] {
        let source = format!("{acquisition}\np File.stat('ignored')\n");
        let parsed = crate::parser::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0, "{source}");
        assert!(
            crate::cops::code_object_safety::Facts::build(&parsed.node())
                .heap
                .observed(),
            "{source}"
        );
    }
    // Other independent guards may still reject wrapped reflective sends. The
    // heap fold must not invent a heap reader for these proved static values.
    for harmless in [
        "Object.send((:to_s))",
        "Object.send((nil))",
        "File.public_send((:stat), 'ignored')",
        "items.map(&((:to_s)))",
        "items.map(&(\"to_s\".to_sym))",
        "items.each(&((nil)))",
    ] {
        let source = format!("{harmless}\np File.stat('ignored')\n");
        let parsed = crate::parser::parse(source.as_bytes());
        assert!(
            !crate::cops::code_object_safety::Facts::build(&parsed.node())
                .heap
                .observed(),
            "{source}"
        );
    }
}

#[test]
fn wrapped_static_harmless_dispatch_and_proven_forwarding_remain_eligible() {
    for harmless in [
        "items.map(&((:to_s)))",
        "((:to_s)).to_proc",
        "items.map(&((:to_s).to_proc))",
        "items.map(&(\"to_s\".to_sym))",
        "items.each(&((nil)))",
        "items.each(&-> { 1 })",
        "def forward(&block); items.each(&((block))); end",
        "Object.send(:to_s)",
        "Object.send(nil)",
        "File.public_send(:stat, 'ignored')",
    ] {
        let source = format!("a = Dir.children('.').empty?\n{harmless}\np File.stat('ignored')\n");
        let found = collect(&crate::cops::style::DirEmpty, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_some(), "{source}");
    }
}

#[test]
fn compound_getters_use_the_same_heap_evidence_as_ordinary_calls() {
    let candidate = "def f(**kw); end\nf(**{a: 1}.merge(b: 2))\n";
    for operator in ["||=", "&&=", "+="] {
        for getter in ["reader.stat", "reader.count_objects", ":stat.to_proc"] {
            let source = format!("{candidate}{getter} {operator} nil\n");
            let found = collect(&KeywordArgumentsMerging, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        let source = format!("{candidate}reader.value {operator} nil\np File.stat('ignored')\n");
        assert!(
            collect(&KeywordArgumentsMerging, &source)[0]
                .correction
                .is_some()
        );
    }
}

#[test]
fn observer_words_in_data_are_not_executable_evidence() {
    for source in [
        "x = 'ObjectSpace GC stat count_objects'\n",
        "# ObjectSpace.count_objects; GC.stat\nx = 'hi'\n",
        "x = 'hi'\n__END__\nObjectSpace.count_objects; GC.stat\n",
    ] {
        let found = collect(&crate::cops::style::StringLiterals, source);
        assert!(found.iter().any(|o| o.correction.is_some()), "{source}");
    }
}

#[test]
fn heap_evidence_is_central_even_for_unregistered_emitter_names() {
    struct Emitter;
    impl Cop for Emitter {
        fn name(&self) -> &'static str {
            "Style/NotInRegistry"
        }
        fn check_source(
            &self,
            _: &crate::source::SourceFile,
            out: &mut Vec<crate::offense::Offense>,
        ) {
            out.push(crate::offense::Offense {
                cop: self.name(),
                severity: self.severity(),
                line: 1,
                end_line: 1,
                col: 1,
                message: "test".into(),
                correction: Some(crate::offense::Correction {
                    range: 0..1,
                    replacement: "2".into(),
                }),
            });
        }
    }
    assert!(collect(&Emitter, "1\np GC.stat\n")[0].correction.is_none());
    assert!(collect(&Emitter, "1\n")[0].correction.is_some());
}
