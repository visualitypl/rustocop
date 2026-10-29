//! `Style/TimeNow` — explicit retrieval of the current system time.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TimeNow;
impl Cop for TimeNow {
    fn name(&self) -> &'static str {
        "Style/TimeNow"
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
        if call.name().as_slice() != b"new"
            || !ast::call_arguments(&call).is_empty()
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Time"]))
        {
            return;
        }
        let start = node.location().start_offset();
        let correction = cx
            .corrections_requested()
            .then(|| {
                let selector = call.message_loc()?;
                let prefix =
                    std::str::from_utf8(&file.bytes[start..selector.start_offset()]).ok()?;
                // Keep the call operator (including &. / ::) verbatim. Literal
                // blocks, comments and line-sensitive edits are withheld by the
                // shared guard; block arguments do not match detection at all.
                // An infix operator or spaced index can become an argument
                // when empty parentheses disappear (e.g. `Time.now +1`).
                // Explicit dot calls do not have that ambiguity.
                let keep_parens = call.opening_loc().is_some()
                    && cx.parent().and_then(|p| p.as_call_node()).is_some_and(|p| {
                        p.call_operator_loc().is_none()
                            && p.receiver().is_some_and(|r| {
                                r.location().start_offset() == start
                                    && r.location().end_offset() == node.location().end_offset()
                            })
                    });
                let parens = if keep_parens { "()" } else { "" };
                super::file_api::replacement(node, cx, file, format!("{prefix}now{parens}"), "Time")
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer `Time.now` over `Time.new` to retrieve the current time.".into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/time_now_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_offenses_and_canonical_corrections() {
        for (before, after) in [
            ("Time.new", "Time.now"),
            ("Time.new()", "Time.now"),
            ("::Time.new", "::Time.now"),
            ("Time&.new", "Time&.now"),
            ("Time.new.year", "Time.now.year"),
        ] {
            let source = format!("# header\n  {before}\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == TimeNow.name())
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Prefer `Time.now` over `Time.new` to retrieve the current time."
            );
            assert!(found[0].correction.is_none());
            assert_correction(&TimeNow, &source, &format!("# header\n  {after}\n"));
        }
    }
    #[test]
    fn empty_parentheses_preserve_operator_and_index_binding() {
        for receiver in ["Time", "::Time"] {
            for operator in [".", "&.", "::"] {
                for suffix in [" +1", " -1", " + 1", " - 1", " [0]", " /2", " **2"] {
                    let before = format!("{receiver}{operator}new(){suffix}");
                    let after = format!("{receiver}{operator}now(){suffix}");
                    assert_correction(&TimeNow, &before, &after);
                }
            }
        }
        assert_correction(&TimeNow, "Time.new().year", "Time.now.year");
    }
    #[test]
    fn pinned_negatives_and_receiver_argument_lookalikes() {
        for source in [
            "Time.now",
            "Time.new(2026, 8, 19)",
            "Time.new(in: '+09:00')",
            "Foo::Time.new",
            "::Foo::Time.new",
            "Date.new",
            "new",
            "time.new",
            "Time.new(&block)",
            "Time.new(*args)",
            "Time.new(**opts)",
            "Time.new(...)",
            "time = object; time.new",
            "# Time.new\n'Time.new'",
        ] {
            // Forwarding requires a forwarding method definition.
            let source = if source == "Time.new(...)" {
                "def f(...); Time.new(...); end"
            } else {
                source
            };
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(correction_offenses(&TimeNow, source).is_empty(), "{source}");
        }
    }
    #[test]
    fn defaults_blocks_and_multiline_source() {
        assert_correction(
            &TimeNow,
            "def f(t = ::Time.new()); t; end",
            "def f(t = ::Time.now); t; end",
        );
        for source in [
            "Time.new { break :value }",
            "Time.new do; raise 'no'; end",
            "Time.new(\n # keep\n)",
            "Time.new(\n)\np __LINE__",
        ] {
            let found = correction_offenses(&TimeNow, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn visible_rebindings_remain_reported_without_edits() {
        for binding in [
            "Time = Custom",
            "module M; Time = Custom; end",
            "class Time; end",
            "def Time.now; :changed; end",
            "Time.define_singleton_method(:new) { :old }",
            "Time.define_method(:initialize) { @marker = :custom; super() }",
            "Time.class_eval { alias_method :initialize, :other }",
            "Time.define_method(:allocate) { :changed }",
            "Class.class_eval('def new; :changed; end')",
            "module M; refine Class do; def new; :changed; end; end; end",
            "Time.singleton_class.class_eval { alias now object_id }",
            "Time.instance_eval('def now; :changed; end')",
            "module M; refine Time.singleton_class do; def now; :changed; end; end; end",
            "Object.const_set(:Time, Custom)",
        ] {
            let source = format!("{binding}\nTime.new\n");
            let found = correction_offenses(&TimeNow, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
