//! `Style/DateTime` — opt-in Time preference, AllowCoercion=false, detection-only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DateTime;
impl Cop for DateTime {
    fn name(&self) -> &'static str {
        "Style/DateTime"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let Some(receiver) = call.receiver() else {
            return;
        };
        let args = ast::call_arguments(&call);
        let coercion = call.name().as_slice() == b"to_datetime" && args.is_empty();
        if !ast::is_global_const(&receiver, &["DateTime"]) && !coercion {
            return;
        }
        if args.len() == 2
            && args[1]
                .as_constant_path_node()
                .and_then(|c| c.parent())
                .is_some_and(|p| ast::is_global_const(&p, &["Date"]))
        {
            return;
        }
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: if coercion {
                "Do not use `#to_datetime`."
            } else {
                "Prefer `Time` over `DateTime`."
            }
            .into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/date_time_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn class_and_explicit_receiver_coercions_are_detection_only() {
        for expression in [
            "DateTime.now",
            "DateTime&.now",
            "::DateTime.now",
            "DateTime.iso8601('2016-06-29')",
            "DateTime.new { side_effect }",
            "thing.to_datetime",
            "thing&.to_datetime",
            "self.to_datetime",
        ] {
            let source = format!("# header\n  {expression}\n");
            let found = correction_offenses(&DateTime, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                if expression.contains("to_datetime") {
                    "Do not use `#to_datetime`."
                } else {
                    "Prefer `Time` over `DateTime`."
                }
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn lexical_constant_rebinding_is_not_inferred_for_detection() {
        let found =
            correction_offenses(&DateTime, "module M; DateTime = Custom; DateTime.now; end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 30));
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn historical_dates_and_lookalikes() {
        for source in [
            "Time.now",
            "Date.iso8601('2016-06-29')",
            "DateTime.iso8601('1751-04-23', Date::ENGLAND)",
            "::DateTime.iso8601('1751-04-23', ::Date::ITALY)",
            "DateTime&.iso8601('1751-04-23', Date::ENGLAND)",
            "Icalendar::Values::DateTime.new(start_at)",
            "def to_time; to_datetime; end",
            "to_datetime = 1; to_datetime",
            "thing.to_datetime(1)",
            "DateTime",
        ] {
            assert!(
                correction_offenses(&DateTime, source).is_empty(),
                "{source}"
            );
        }
    }
}
