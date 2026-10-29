//! `Style/StderrPuts` — prefer suppressible warning output.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StderrPuts;
impl Cop for StderrPuts {
    fn name(&self) -> &'static str {
        "Style/StderrPuts"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.name().as_slice() != b"puts"
            || ast::call_arguments(&call).is_empty()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if !ast::is_global_const(&receiver, &["STDERR"])
            && !receiver
                .as_global_variable_read_node()
                .is_some_and(|g| g.name().as_slice() == b"$stderr")
        {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let start = call.location().start_offset();
        let end = call.message_loc().unwrap().end_offset();
        let (line, col) = file.line_col(start);
        let bad = format!(
            "{}.puts",
            String::from_utf8_lossy(receiver.location().as_slice())
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `warn` instead of `{bad}` to allow such output to be disabled."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: (cx.corrections_requested()
                && call.block().is_none()
                && !file.bytes[start..end]
                    .iter()
                    .any(|b| matches!(b, b'\n' | b'\r' | b'#'))
                && arguments.iter().all(|arg| {
                    arg.as_string_node().is_some_and(|s| {
                        !s.opening_loc()
                            .is_some_and(|l| l.as_slice().starts_with(b"<<"))
                            // warn concatenates arguments; puts writes them separately.
                            // Non-ASCII literals can carry incompatible encodings even
                            // within one source file (e.g. byte and Unicode escapes).
                            && (arguments.len() == 1 || s.unescaped().is_ascii())
                    })
                })
                && file
                    .stderr_bindings(cx.ancestors().last().unwrap_or(node))
                    .can_correct(start))
            .then(|| Correction {
                range: start..end,
                replacement: "warn".into(),
            }),
        });
    }
}
/// Conservative file-wide evidence, deliberately not a value-flow simulation.
/// Unknown external monkey patches are not inferable; visible custom bindings,
/// redirections and metaprogramming withhold edits without hiding findings.
#[derive(Default)]
pub(crate) struct Bindings {
    pub(crate) unsafe_binding: bool,
    verbosity_writes: usize,
    proven_calls: Vec<usize>,
    #[cfg(test)]
    pub(crate) visited: usize,
    #[cfg(test)]
    pub(crate) prefix_visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self::default();
        crate::cops::walk::walk(&mut result, root);
        result.prove_non_nil_prefix(root);
        result
    }
    fn can_correct(&self, offset: usize) -> bool {
        !self.unsafe_binding && self.proven_calls.binary_search(&offset).is_ok()
    }
    /// No runtime inference: accept only a literal non-nil assignment as the
    /// first top-level statement, followed by contiguous literal stderr sends
    /// (optionally assigned to a local). Stop at *any* other statement: calls,
    /// branches, definitions and deferred scopes cannot establish dominance.
    /// Any other verbosity write anywhere, including a deferred method/block,
    /// invalidates the proof. This linear prefix pass shares the lazy cache;
    /// individual offenses only do a binary search, never a sibling/tree scan.
    fn prove_non_nil_prefix(&mut self, root: &Node<'_>) {
        if self.unsafe_binding || self.verbosity_writes != 1 {
            return;
        }
        let Some(program) = root.as_program_node() else {
            return;
        };
        let statements = program.statements();
        let mut body = statements.body().iter();
        let Some(first) = body.next() else { return };
        let Some(write) = first.as_global_variable_write_node() else {
            return;
        };
        if write.name().as_slice() != b"$VERBOSE"
            || !matches!(
                write.value(),
                Node::FalseNode { .. } | Node::TrueNode { .. }
            )
        {
            return;
        }
        for statement in body {
            #[cfg(test)]
            {
                self.prefix_visited += 1;
            }
            let value = statement.as_local_variable_write_node().map(|n| n.value());
            let node = value.as_ref().unwrap_or(&statement);
            let Some(call) = node.as_call_node() else {
                break;
            };
            if call.is_safe_navigation()
                || call.name().as_slice() != b"puts"
                || call.block().is_some()
                || !call.receiver().is_some_and(|receiver| {
                    ast::is_global_const(&receiver, &["STDERR"])
                        || receiver
                            .as_global_variable_read_node()
                            .is_some_and(|g| g.name().as_slice() == b"$stderr")
                })
            {
                break;
            }
            let args = ast::call_arguments(&call);
            // No evaluation/coercion or block dispatch may intervene between
            // the assignment and a later correction in the prefix.
            if args.is_empty() || !args.iter().all(|arg| arg.as_string_node().is_some()) {
                break;
            }
            self.proven_calls.push(call.location().start_offset());
        }
    }
    fn protect_constant(&mut self, name: &[u8]) {
        self.unsafe_binding |= matches!(name, b"STDERR" | b"Kernel" | b"IO" | b"Warning");
    }
    fn protect_global(&mut self, name: &[u8]) {
        self.unsafe_binding |= name == b"$stderr";
        // Ruby's writable verbosity aliases change the same interpreter state.
        self.verbosity_writes += usize::from(matches!(name, b"$VERBOSE" | b"$-v" | b"$-w"));
    }
}
const METHODS: &[&[u8]] = &[b"warn", b"puts", b"write"];
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Bindings {
    fn enter(&mut self, node: Node<'pr>) {
        use super::file_api::{binds_constant, binds_method, may_name_method};
        #[cfg(test)]
        {
            self.visited += 1;
        }
        macro_rules! constants {
            ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.protect_constant(n.name().as_slice()); })* };
        }
        constants!(
            as_constant_write_node,
            as_constant_or_write_node,
            as_constant_and_write_node,
            as_constant_operator_write_node,
            as_constant_target_node
        );
        macro_rules! globals {
            ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.protect_global(n.name().as_slice()); })* };
        }
        globals!(
            as_global_variable_write_node,
            as_global_variable_or_write_node,
            as_global_variable_and_write_node,
            as_global_variable_operator_write_node,
            as_global_variable_target_node
        );
        if let Some(n) = node.as_constant_path_target_node() {
            self.protect_constant(n.name_loc().as_slice());
        }
        self.unsafe_binding |= matches!(node, Node::AliasGlobalVariableNode { .. });
        // Path writes use a ConstantPathNode target in Prism. Conservatively
        // include reads too: they indicate use of these mutable core APIs.
        // BasicObject lacks Kernel#warn, even with no visible override.
        // Ractor execution can make even reading STDERR raise, unlike warn.
        // Withhold file-wide: methods called inside a Ractor need not be
        // lexically nested in its block.
        if let Some(n) = node.as_constant_path_node()
            && n.name().is_some_and(|n| {
                matches!(
                    n.as_slice(),
                    b"Kernel" | b"IO" | b"Warning" | b"BasicObject" | b"Ractor"
                )
            })
        {
            self.unsafe_binding = true;
        }
        macro_rules! paths {
            ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.protect_constant(n.target().name().map_or(b"".as_slice(), |n| n.as_slice())); })* };
        }
        paths!(
            as_constant_path_write_node,
            as_constant_path_or_write_node,
            as_constant_path_and_write_node,
            as_constant_path_operator_write_node
        );
        if let Some(n) = node.as_constant_read_node()
            && matches!(
                n.name().as_slice(),
                b"Kernel" | b"IO" | b"Warning" | b"BasicObject" | b"Ractor"
            )
        {
            self.unsafe_binding = true;
        }
        if let Some(n) = node.as_def_node() {
            self.unsafe_binding |= METHODS.contains(&n.name().as_slice());
        }
        if let Some(n) = node.as_alias_method_node() {
            self.unsafe_binding |= may_name_method(&n.new_name(), METHODS);
        }
        if let Some(n) = node.as_undef_node() {
            self.unsafe_binding |= n.names().iter().any(|n| may_name_method(&n, METHODS));
        }
        let locals = match node {
            Node::ProgramNode { .. } => Some(node.as_program_node().unwrap().locals()),
            Node::DefNode { .. } => Some(node.as_def_node().unwrap().locals()),
            Node::BlockNode { .. } => Some(node.as_block_node().unwrap().locals()),
            Node::LambdaNode { .. } => Some(node.as_lambda_node().unwrap().locals()),
            Node::ClassNode { .. } => Some(node.as_class_node().unwrap().locals()),
            Node::ModuleNode { .. } => Some(node.as_module_node().unwrap().locals()),
            Node::SingletonClassNode { .. } => {
                Some(node.as_singleton_class_node().unwrap().locals())
            }
            _ => None,
        };
        self.unsafe_binding |= locals.is_some_and(|l| l.iter().any(|n| n.as_slice() == b"warn"));
        if let Some(call) = node.as_call_node() {
            self.unsafe_binding |= binds_method(&call, METHODS)
                || [b"STDERR".as_slice(), b"Kernel", b"IO", b"Warning"]
                    .iter()
                    .any(|name| binds_constant(&call, name))
                || matches!(
                    call.name().as_slice(),
                    b"send"
                        | b"__send__"
                        | b"public_send"
                        | b"reopen"
                        | b"close"
                        | b"refine"
                        | b"using"
                        | b"eval"
                        | b"module_eval"
                        | b"class_eval"
                        | b"instance_eval"
                        | b"module_exec"
                        | b"class_exec"
                        | b"instance_exec"
                );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/stderr_puts_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_receivers_and_canonical_corrections() {
        for receiver in ["$stderr", "STDERR", "::STDERR"] {
            let source = format!("# header\n  {receiver}.puts('hello')\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/StderrPuts")
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `warn` instead of `{receiver}.puts` to allow such output to be disabled."
                )
            );
            // The upstream canonical edit is unsafe with interpreter-configured
            // nil verbosity. Keep its detection, withhold its correction.
            assert!(
                correction_offenses(&StderrPuts, &source)[0]
                    .correction
                    .is_none()
            );
            let proven = format!("$VERBOSE = false\n{source}");
            assert_correction(
                &StderrPuts,
                &proven,
                "$VERBOSE = false\n# header\n  warn('hello')\n",
            );
        }
        assert_correction(
            &StderrPuts,
            "$VERBOSE = true\n$stderr.puts \"hello\", \"world\"\n",
            "$VERBOSE = true\nwarn \"hello\", \"world\"\n",
        );
    }
    #[test]
    fn only_the_uninterrupted_top_level_prefix_has_proven_verbosity() {
        for verbose in ["false", "true"] {
            let source = format!(
                "$VERBOSE = {verbose}\n$stderr.puts('first')\nresult = STDERR.puts('second')\np result\n::STDERR.puts('unproven')\n"
            );
            let found = correction_offenses(&StderrPuts, &source);
            assert_eq!(found.len(), 3);
            assert!(found[0].correction.is_some());
            assert!(found[1].correction.is_some());
            assert!(found[2].correction.is_none());
            assert_correction(
                &StderrPuts,
                &source,
                &source
                    .replacen("$stderr.puts", "warn", 1)
                    .replacen("STDERR.puts", "warn", 1),
            );
        }
        for source in [
            "$stderr.puts('x')\n$VERBOSE = false",
            "p :before\n$VERBOSE = false\n$stderr.puts('x')",
            "$VERBOSE = false\nif true; $stderr.puts('x'); end",
            "$VERBOSE = false\ndef f; $stderr.puts('x'); end",
            "$VERBOSE = false\n-> { $stderr.puts('x') }.call",
            "$VERBOSE = false\nbegin; $stderr.puts('x'); ensure; end",
            "$VERBOSE = false\n$stderr.puts('x')\ndef quiet; $VERBOSE = nil; end",
            "$VERBOSE = false\n$stderr.puts('x')\n[1].each { $VERBOSE = nil }",
            "$VERBOSE = false\n$stderr.puts('x')\n$VERBOSE, local = nil, 1",
            "$VERBOSE = false\n$stderr.puts('x')\n$VERBOSE ||= true",
            "$VERBOSE = false\n$stderr.puts('x')\n$-v = nil",
            "$VERBOSE = false\n$stderr.puts('x')\n$-w = nil",
            "$VERBOSE = false\n$stderr.puts('x')\nalias $quiet $VERBOSE",
        ] {
            let found = correction_offenses(&StderrPuts, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn proven_verbosity_does_not_bypass_file_wide_stream_and_binding_guards() {
        for suffix in [
            "$stderr = custom",
            "STDERR = Custom",
            "STDERR.reopen(path)",
            "def warn(*); end",
            "warn = custom",
            "alias warn custom",
            "undef warn",
            "module Kernel; end",
            "class IO; end",
            "Warning[:deprecated] = true",
            "using Custom",
            "Ractor.new {}.take",
            "::Ractor.new {}.take",
            "def f; ::Ractor.new {}.take; end",
            "alias $stderr $stdout",
        ] {
            let source = format!("$VERBOSE = false\n$stderr.puts('x')\n{suffix}\n");
            let found = correction_offenses(&StderrPuts, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn proven_verbosity_retains_literal_and_mixed_encoding_guards() {
        for (encoding, args) in [
            ("ISO-8859-1", "\"\\xE9\", \"\\u00E9\""),
            ("ASCII-8BIT", "\"\\u00E9\", \"\\xE9\""),
            ("UTF-8", "dynamic"),
            ("UTF-8", "*args"),
            ("UTF-8", "\"#{effect}\""),
            ("UTF-8", "[1, 2]"),
        ] {
            let source = format!("# encoding: {encoding}\n$VERBOSE = true\n$stderr.puts({args})\n");
            let found = correction_offenses(&StderrPuts, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn uncertain_edits_stay_reported_without_rewriting() {
        for source in [
            "# encoding: ISO-8859-1\n$stderr.puts(\"\\xE9\", \"\\u00E9\")",
            "# encoding: ASCII-8BIT\n$stderr.puts(\"\\u00E9\", \"\\xE9\")",
            "Ractor.new { STDERR.puts('hello') }.take",
            "::Ractor.new { ::STDERR.puts('hello') }.take",
            "def output; STDERR.puts('hello'); end; Ractor.new { output }.take",
            "$stderr.puts(dynamic)",
            "$stderr.puts(*args)",
            "$stderr.puts(**opts)",
            "$stderr.puts(&block)",
            "$stderr.puts(\"#{effect}\")",
            "$stderr.puts([1, 2])",
            "$stderr.puts(Object.new)",
            "$stderr.puts('x') { break :changed }",
            "$stderr.puts('x', &block)",
            "$stderr. # retain comment\n puts('x')",
            "$stderr.\n puts('x')\np __LINE__",
            "$stderr.puts(<<~TEXT)\nx\nTEXT\n",
            "$stderr = custom; STDERR.puts('x')",
            "$stderr = custom; $stderr.puts('x')",
            "STDERR = Custom; STDERR.puts('x')",
            "Other::STDERR = Custom; ::STDERR.puts('x')",
            "$stderr.reopen(path); $stderr.puts('x')",
            "STDERR.close; STDERR.puts('x')",
            "$VERBOSE = nil; $stderr.puts('x')",
            "$VERBOSE = setting; $stderr.puts('x')",
            "def warn(*); :custom; end; $stderr.puts('x')",
            "warn = custom; $stderr.puts 'x'",
            "def puts(*); end; $stderr.puts('x')",
            "def write(*); end; $stderr.puts('x')",
            "module Kernel; end; $stderr.puts('x')",
            "class IO; end; STDERR.puts('x')",
            "module R; refine Kernel do; def warn(*); end; end; end; using R; $stderr.puts('x')",
            "alias warn custom; $stderr.puts('x')",
            "undef warn; $stderr.puts('x')",
            "define_method(:warn) {}; $stderr.puts('x')",
            "attr_reader :warn; $stderr.puts 'x'",
            "Kernel.send(:define_method, :warn) {}; $stderr.puts('x')",
            "Kernel.const_set(:STDERR, Custom); $stderr.puts('x')",
            "Kernel.module_eval('def warn(*); end'); $stderr.puts('x')",
            "Kernel.module_eval { define_method(:warn) {} }; $stderr.puts('x')",
            "class Bare < BasicObject; def f; $stderr.puts('x'); end; end",
            "alias $stderr $stdout; $stderr.puts('x')",
            "STDERR.send(:reopen, path); STDERR.puts('x')",
            "::Other::STDERR ||= Custom; STDERR.puts('x')",
            "::Other::STDERR &&= Custom; STDERR.puts('x')",
            "::Other::STDERR += Custom; STDERR.puts('x')",
        ] {
            let found = correction_offenses(&StderrPuts, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn byte_local_replacements_preserve_bom_crlf_encoding_data_and_line_values() {
        let before = "\u{feff}# encoding: UTF-8\r\n$VERBOSE = false\r\n$stderr.puts(\"café\\n\") # retained\r\np __LINE__\r\n__END__\r\n$stderr.puts('raw')\r\n";
        let after = before.replacen("$stderr.puts", "warn", 1);
        assert_correction(&StderrPuts, before, &after);
        assert_correction(
            &StderrPuts,
            "$VERBOSE = false; STDERR::puts('x')",
            "$VERBOSE = false; warn('x')",
        );
    }
    #[test]
    fn pinned_no_argument_calls_and_receiver_lookalikes() {
        for source in [
            "$stderr.puts",
            "STDERR.puts",
            "::STDERR.puts()",
            "$stdout.puts('x')",
            "$stdin.puts('x')",
            "Other::STDERR.puts('x')",
            "stderr.puts('x')",
            "$stderr&.puts('x')",
            "$stderr.print('x')",
            "puts('x')",
        ] {
            assert!(
                correction_offenses(&StderrPuts, source).is_empty(),
                "{source}"
            );
        }
    }
}
