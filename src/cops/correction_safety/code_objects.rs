//! Independent synthetic main-Prism evidence under the fixed Ruby 3.4 policy.
//! Explicit version-3.4 parser and host-VM evidence live in separate CLI tests.
use super::collect;
use crate::cops::layout::SpaceAroundOperators;

#[test]
fn code_object_acquisition_and_uncertainty_withhold_even_same_size_edits() {
    for observer in [
        "p ::RubyVM::AbstractSyntaxTree.of(-> {}).first_column",
        "p Object::RubyVM::InstructionSequence.of(-> {}).to_a",
        "AST = RubyVM::AbstractSyntaxTree; p AST.of(-> {}).children",
        "VM = RubyVM; p VM::AbstractSyntaxTree.of(-> {}).type",
        "def later; RubyVM::AbstractSyntaxTree.of(-> {}).first_column; end; p later",
        "factory = RubyVM::AbstractSyntaxTree.method(:of); p factory.call(-> {}).type",
        "p RubyVM::AbstractSyntaxTree.public_send(:send, :of, -> {}).children",
        "class << RubyVM::InstructionSequence; alias acquire of; end; p RubyVM::InstructionSequence.acquire(-> {}).to_a",
        "p RubyVM.const_get(:AbstractSyntaxTree).of(-> {}).type",
        "p Object.const_get('RubyVM::InstructionSequence').of(-> {}).to_a",
        "p Object.public_send(:const_get, 'RubyVM').const_get(:AbstractSyntaxTree)",
        "p Object.method(:const_get).call(name)",
        "p Module.instance_method(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.public_instance_method(:const_get).bind_call(Object, 'RubyVM::AbstractSyntaxTree').of(-> {}).first_column",
        "p Module.public_send(:send, :instance_method, :const_get).bind_call(Object, name)",
        "p Module.send(:public_send, :public_instance_method, :const_get).bind_call(Object, name)",
        "p Module.method(:instance_method).call(:const_get).bind_call(Object, name)",
        "p Module.method(:public_instance_method).call(:const_get).bind_call(Object, name)",
        "getter = Module.instance_method(selector); p getter.bind_call(Object, name)",
        "getter = Module.public_instance_method(selector); p getter.bind_call(Object, name)",
        "getter = :instance_method.to_proc; p getter.call(Module, :const_get)",
        "getter = :public_instance_method.to_proc; p getter.call(Module, :const_get)",
        "getter = :const_get.to_proc; p getter.call(Object, name)",
        "p Object.const_get(name)",
        "p Object.const_get(:CodeProvider)",
        "p Provider.const_get(:String)",
        "p RubyVM::YJIT.runtime_stats",
        "p RubyVM.stat(:constant_cache_misses)",
        "p ::RubyVM.stat",
        "p Object.public_send(selector, name)",
        "O = Object; p O.public_send(selector, name)",
        "lookup = Object.method(:public_send); p lookup.call(:const_get, name)",
        "lookup = :send.to_proc; p lookup.call(Object, :const_get, name)",
        "class << Object; alias lookup send; end; p Object.lookup(:const_get, name)",
        "p Object.public_send(selector).call(:String)",
        "p RubyVM::UnknownFactory.stat",
        "p RubyVM::UnknownFactory::OPTS",
        "p RubyVM.public_send(selector)",
        "p RubyVM.unknown_factory",
        "p RubyVM::UnknownFactory.acquire",
        "escape(RubyVM::InstructionSequence)",
        "RubyVM.keep_script_lines = true; p RubyVM::InstructionSequence.compile_file(__FILE__).script_lines",
        "p RubyVM::AbstractSyntaxTree.parse_file(__FILE__).children",
    ] {
        let source = format!("x=1\n{observer}\n");
        let offenses = collect(&SpaceAroundOperators, &source);
        assert_eq!(offenses.len(), 1, "{source}");
        assert_eq!((offenses[0].line, offenses[0].col), (1, 2));
        assert_eq!(
            offenses[0].message,
            "Surrounding space missing for operator `=`."
        );
        assert!(offenses[0].correction.is_none(), "{source}");
        // Quote normalization changes neither bytes nor physical lines.
        let same_size = format!("x = 'hi'\n{observer}\n");
        let found = collect(&crate::cops::style::StringLiterals, &same_size);
        assert!(!found.is_empty());
        assert!(found.iter().all(|o| o.correction.is_none()), "{same_size}");
    }
}

#[test]
fn serialized_code_factory_acquisition_includes_namespace_aliases_and_unknown_selectors() {
    for observer in [
        r#"k=Marshal.load("\x04\x08\x6d\x1f\x52\x75\x62\x79\x56\x4d\x3a\x3a\x41\x62\x73\x74\x72\x61\x63\x74\x53\x79\x6e\x74\x61\x78\x54\x72\x65\x65"); p k.of(-> {}).first_column"#,
        "k = ::Marshal.restore(payload); p k.of(-> {}).children",
        "k = Object::Marshal.public_send(:send, :restore, payload)",
        "M = Marshal; k = M.load(payload)",
        "def later; Marshal.load(payload); end; k = later",
        "getter = Marshal.method(:load); k = getter.call(payload)",
        "getter = Marshal.public_method(:restore); k = getter.call(payload)",
        "k = Marshal.singleton_class.instance_method(:load).bind_call(Marshal, payload)",
        "k = :load.to_proc.call(Marshal, payload)",
        "k = [Marshal].map(&:restore)",
        "class << Marshal; alias acquire load; end; k = Marshal.acquire(payload)",
        "class << Marshal; alias_method :acquire, :restore; end; k = Marshal.acquire(payload)",
        "k = Marshal.public_send(selector, payload)",
        "escape(Marshal)",
        "k = Object.const_get(:Marshal).load(payload)",
        "k = Other::Marshal.dump(value)",
        "k = self::Marshal.dump(value)",
    ] {
        let source = format!("x=1\n{observer}\n");
        let found = collect(&SpaceAroundOperators, &source);
        assert!(!found.is_empty(), "{source}");
        assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        let source = format!("x = 'hi'\n{observer}\n");
        let found = collect(&crate::cops::style::StringLiterals, &source);
        assert!(!found.is_empty(), "{source}");
        assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
    }
}

#[test]
fn compound_constant_reads_are_factory_acquisitions_not_just_namespace_mutations() {
    for name in [
        "Marshal",
        "RubyVM",
        "AbstractSyntaxTree",
        "InstructionSequence",
    ] {
        for prefix in ["", "::", "Object::", "Other::"] {
            for operator in ["||=", "&&=", "+="] {
                let observer = format!("k = ({prefix}{name} {operator} nil); escape(k)");
                let source = format!("x=1\n{observer}\n");
                let parsed = crate::parser::parse(source.as_bytes());
                assert_eq!(parsed.errors().count(), 0, "{source}");
                let facts = crate::cops::code_object_safety::Facts::build(&parsed.node());
                assert!(facts.acquired_or_uncertain, "{source}");
                assert_eq!(
                    facts.visits,
                    crate::cops::testing::preorder_kinds(&source).len()
                );
                assert!(facts.constant_segments <= facts.visits);
                let found = collect(&SpaceAroundOperators, &source);
                assert!(!found.is_empty(), "{source}");
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
                let source = format!("x = 'hi'\n{observer}\n");
                let found = collect(&crate::cops::style::StringLiterals, &source);
                assert!(!found.is_empty(), "{source}");
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            }
        }
    }
    // A compound write is not a global correction ban. Unrelated constants and
    // inert spellings retain ordinary fixes; existing metadata controls remain.
    for observer in [
        "n = (Other ||= nil)",
        "n = (Other &&= nil)",
        "n = (Other += 1)",
        "p '(Marshal ||= nil).load(payload)'",
        "# (Marshal ||= nil).restore(payload)",
    ] {
        let source = format!("x=1\n{observer}\n");
        assert!(
            collect(&SpaceAroundOperators, &source)[0]
                .correction
                .is_some(),
            "{source}"
        );
    }
}

#[test]
fn serializer_non_acquiring_controls_require_unchanged_core_dispatch() {
    for observer in [
        "p Marshal.name",
        "p ::Marshal.class",
        "p Object::Marshal.object_id",
        "p defined?(Marshal)",
        "p Marshal.dump(42)",
        "p ::Marshal.dump('hi')",
        "p Marshal::MAJOR_VERSION",
        "p Marshal::MINOR_VERSION",
    ] {
        let source = format!("x=1\n{observer}\n");
        let parsed = crate::parser::parse(source.as_bytes());
        assert!(
            !crate::cops::code_object_safety::Facts::build(&parsed.node()).acquired_or_uncertain,
            "{source}"
        );
        // dump already crosses the independent heap-reader guard; this task
        // neither removes that hold nor mislabels it as code acquisition.
        assert_eq!(
            collect(&SpaceAroundOperators, &source)[0]
                .correction
                .is_some(),
            !observer.contains("dump("),
            "{source}"
        );
        for mutation in [
            "def Marshal.dump(*); acquire; end",
            "def name; acquire; end",
            "Marshal = provider",
            "Object.const_set(:Marshal, provider)",
            "Marshal.singleton_class.prepend(provider)",
        ] {
            let source = format!("x=1\n{observer}\n{mutation}\n");
            assert!(
                collect(&SpaceAroundOperators, &source)
                    .iter()
                    .all(|o| o.correction.is_none()),
                "{source}"
            );
        }
    }
    for observer in [
        "p 'Marshal.load(payload)'",
        "# Marshal.restore(payload)",
        "p YAML.safe_load('42')",
        "p YAML.load(File.read(File::NULL))",
    ] {
        let source = format!("x=1\n{observer}\n");
        assert!(
            collect(&SpaceAroundOperators, &source)[0]
                .correction
                .is_some(),
            "{source}"
        );
    }
}

#[test]
fn all_core_exemptions_require_unchanged_main_ast_dispatch() {
    let mutations = [
        "class << Object; def const_get(name); super('RubyVM::AbstractSyntaxTree'); end; end",
        "def const_get(name); super('RubyVM::AbstractSyntaxTree'); end",
        "def name; acquire; end",
        "def class; acquire; end",
        "def object_id; acquire; end",
        "def enabled?; acquire; end",
        "def call(*args); acquire; end",
        "def method(name); acquire; end",
        "def public_method(name); acquire; end",
        "def to_proc; acquire; end",
        "def eval(code); acquire; end",
        "def binding; acquire; end",
        "def method_added(name); acquire; end",
        "alias acquire eval",
        "alias_method :const_get, :acquire",
        "Object.define_singleton_method(:const_get) { |name| acquire }",
        "Module.define_method(:const_get) { |name| acquire }",
        "attr_reader :const_get",
        "private :const_get",
        "public_class_method :const_get",
        "private_constant :String",
        "remove_method :const_get",
        "undef const_get",
        "Object.singleton_class.prepend(provider)",
        "include provider",
        "extend provider",
        "refine Object do; end",
        "using provider",
        "Object.public_send(:send, :define_singleton_method, :const_get) { acquire }",
        "Object.method(:remove_method).call(:const_get)",
        "Object.send(selector, :const_get)",
        "Object.const_set(:String, provider)",
        "Object.remove_const(:String)",
        "String = provider",
        "Object::String ||= provider",
        "String, Other = providers",
        "autoload :String, 'provider'",
        "require 'provider'",
        "load 'provider.rb'",
        "Object.class_exec { mutate }",
        "Object.class_eval { mutate }",
        "Object.instance_eval { mutate }",
        "Object.module_eval { mutate }",
        "BEGIN { mutate }",
        "Object.public_send(:send, :alias_method, :getter, :other)",
        "Object.method(:define_singleton_method).call(:const_get) { acquire }",
        "Object.send(:private_constant, :String)",
        "Object.send(:remove_const, :String)",
        "Object.send(:const_set, :String, provider)",
        "Object.send(:prepend, provider)",
        "Object.send(:using, provider)",
        "Object.method(:public_class_method).call(:const_get)",
        "Object.public_send(:send, :remove_method, :const_get)",
        "Object::String &&= provider",
        "Object::String += provider",
        "Object::String, Other = providers",
    ];
    for observer in [
        "p RubyVM.name",
        "p RubyVM::OPTS",
        "p RubyVM::DEFAULT_PARAMS",
        "p RubyVM::YJIT.enabled?",
        "p RubyVM::AbstractSyntaxTree.class",
        "p RubyVM::InstructionSequence.object_id",
        "p defined?(RubyVM::InstructionSequence)",
        "p Object.const_get(:String).of(-> {}).first_column",
        "p Object.public_send(:send, :const_get, :String).name",
        "p Object.method(:const_get).call(:String).of(-> {}).type",
        "p eval('42', binding)",
    ] {
        for mutation in mutations {
            // Both orders: a later/deferred mutation also ends the proof.
            for tail in [
                format!("{mutation}\n{observer}"),
                format!("{observer}\n{mutation}"),
            ] {
                let source = format!("x=1\n{tail}\n");
                let found = collect(&SpaceAroundOperators, &source);
                assert!(!found.is_empty(), "{source}");
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            }
        }
    }
}

#[test]
fn unrelated_and_invalid_reflective_selectors_also_require_core_method_dispatch() {
    for getter in [
        "Object.method(:identity).call('RubyVM::AbstractSyntaxTree')",
        "Object.method(123).call('RubyVM::AbstractSyntaxTree')",
        "Object.send(123)",
        "Object.send(:name)",
        "Object.public_send(:name)",
        ":identity.to_proc",
        "items.map(&:identity)",
    ] {
        for mutation in [
            "def method(name); super(:const_get); end",
            "def send(name); super(:const_get, 'RubyVM::AbstractSyntaxTree'); end",
            "def to_proc; acquire; end",
            "Object.define_singleton_method(:method) { acquire }",
            "Object.send(:alias_method, :method, :provider)",
            "Object.method(:remove_method).call(:method)",
            "Object.singleton_class.prepend(provider)",
            "require 'provider'",
            "load 'provider.rb'",
        ] {
            let source = format!("x=1\n{mutation}\np {getter}\n");
            let found = collect(&SpaceAroundOperators, &source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
}

#[test]
fn evaluation_and_untyped_factory_acquisition_are_uncertain_without_parsing_strings() {
    for observer in [
        "K=eval('RubyVM::AbstractSyntaxTree'); p K.of(-> {}).first_column",
        "K=eval(code); p K.of(-> {}).first_column",
        "K=binding.eval(code)",
        "K=Kernel.public_send(:send, :eval, code)",
        "K=Object.module_eval(code)",
        "K=Object.class_eval(code)",
        "K=instance_eval(code)",
        "K=method(:eval).call(code)",
        "K=Kernel.method(:public_send).call(:eval, code)",
        "K=Binding.instance_method(:eval).bind_call(binding, code)",
        "alias acquire eval; K=acquire(code)",
        "alias_method :acquire, :eval; K=acquire(code)",
        "Object.send(:send, :alias_method, :acquire, :eval); K=acquire(code)",
        "Object.method(:alias_method).call(:acquire, :eval); K=acquire(code)",
        "ObjectSpace.method(:_id2ref).call(id)",
        "Object.module_eval(&code)",
        "eval(code) { 42 }",
        "p Other::RubyVM.name",
        "p self::RubyVM.name",
        "p RubyVM.name(argument)",
        "K=:eval.to_proc.call(Kernel, code)",
        "K=[code].map(&method(:eval))",
        "def later; eval(code); end; K=later",
        "K=-> { eval(code) }; p K.call.of(-> {}).type",
        "K=ObjectSpace._id2ref(id); p K.of(-> {}).type",
        "K=ObjectSpace.each_object(Class).find { |c| predicate(c) }; p K.of(-> {}).type",
        "K=Object.const_missing(:Factory)",
        "K=Object.method(:const_missing).call(:Factory)",
    ] {
        let source = format!("x=1\n{observer}\n");
        let found = collect(&SpaceAroundOperators, &source);
        assert!(!found.is_empty(), "{source}");
        assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
    }
}

#[test]
fn literal_block_evaluation_requires_unchanged_evaluator_dispatch() {
    for evaluator in [
        "class_eval",
        "module_eval",
        "instance_eval",
        "class_exec",
        "module_exec",
        "instance_exec",
    ] {
        for mutation in [
            format!("def Object.{evaluator}(*); super('RubyVM::AbstractSyntaxTree', &nil); end"),
            format!("Object.define_singleton_method(:{evaluator}) {{ acquire }}"),
            format!("alias {evaluator} acquire"),
            format!("alias_method :{evaluator}, :acquire"),
            format!("Object.send(:define_singleton_method, :{evaluator}) {{ acquire }}"),
            "def Object.send(*); acquire; end".into(),
            "Object.singleton_class.prepend(provider)".into(),
        ] {
            for observer in [
                format!("k = Object.{evaluator} {{ 1 }}"),
                format!("k = Object.send(:{evaluator}) {{ 1 }}"),
            ] {
                for source in [
                    format!("{mutation}\n{observer}\nx=1\n"),
                    format!("{observer}\nx=1\n{mutation}\n"),
                ] {
                    let found = collect(&SpaceAroundOperators, &source);
                    assert!(!found.is_empty(), "{source}");
                    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
                }
            }
        }
    }
}

#[test]
fn maximal_constant_paths_are_inspected_once_without_prefix_materialization() {
    // Each segment appears in exactly one maximal path; increasing both width
    // and depth must not produce triangular prefix work. These are parse-only.
    for depth in [1, 8, 40] {
        let path = format!("{}RubyVM::AbstractSyntaxTree", "Outer::".repeat(depth));
        let source = format!("p {path}.name\n").repeat(100);
        let parsed = crate::parser::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let facts = crate::cops::code_object_safety::Facts::build(&parsed.node());
        assert!(facts.acquired_or_uncertain); // Not a proven core namespace.
        assert_eq!(facts.constant_segments, 100 * (depth + 2));
        assert_eq!(
            facts.visits,
            crate::cops::testing::preorder_kinds(&source).len()
        );
    }
}

#[test]
fn proven_namespace_and_non_code_metadata_do_not_block_canonical_fixes() {
    for observer in [
        "p RubyVM.name",
        "p RubyVM::OPTS",
        "p RubyVM::DEFAULT_PARAMS",
        "p RubyVM::YJIT.enabled?",
        "p RubyVM::InstructionSequence.name",
        "p RubyVM::AbstractSyntaxTree.class",
        "p defined?(RubyVM::InstructionSequence)",
        "p Object.const_get(:String).name",
        "p Object.public_send(:send, :const_get, :String).name",
        "p Object.method(:const_get).call(:String).name",
        "def identity(x); x; end; p method(:identity).call(1)",
        "def identity(x); x; end; p Object.const_get(:String).name",
        "def unrelated; 1; end; p RubyVM.name",
        "p eval('42', binding)",
        "p Kernel.eval('nil')",
        "p Object.class_eval('true')",
        "p Module.module_eval('false')",
        "p instance_eval('0')",
        "p Object.class_eval { 1 }",
        "p Object.module_eval { 1 }",
        "p Object.instance_eval { 1 }",
        "p Object.send(:class_eval) { 1 }",
        "Object.class_eval { def unrelated; 1; end; alias other unrelated }",
        "Object.module_eval { alias_method :other, :unrelated }",
        "Object.instance_eval { @value = 1 }",
        "Object.class_eval { Object.instance_exec { 1 } }",
        "Object.module_exec { alias_method :other, :unrelated }",
        "Object.instance_exec { @value = 1 }",
        "Object.class_exec { def unrelated; 1; end }",
        "p 'RubyVM::AbstractSyntaxTree.of'",
        "p 'eval(code)'",
        "# Object.define_singleton_method(:const_get) { acquire }",
        "# RubyVM::InstructionSequence.of(-> {})",
        "def first_column; 1; end; p first_column",
    ] {
        let source = format!("x=1\n{observer}\n");
        let found = collect(&SpaceAroundOperators, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_some(), "{source}");
    }
}
