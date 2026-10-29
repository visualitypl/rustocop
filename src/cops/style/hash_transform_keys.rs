//! `Style/HashTransformKeys` — unsafe, detection-only hash transformation advice.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HashTransformKeys;
impl Cop for HashTransformKeys {
    fn name(&self) -> &'static str {
        "Style/HashTransformKeys"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        super::hash_transform::check(self, true, node, cx, file, out);
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop hash_transform_keys_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    #[test]
    fn exact_locations_messages_and_no_fixes() {
        for (s, desc, line, col, end) in [
            (
                "# comment\n  {}.each_with_object({}) do |(k,v),h|\n    h[foo(k)] = v\n  end",
                "each_with_object",
                2,
                3,
                4,
            ),
            (
                "wrap do\n  {}.map { |k,v| [k.to_s,v] }.\n    to_h\nend",
                "map {...}.to_h",
                2,
                3,
                3,
            ),
            (
                "  Hash[{}.collect { |k,v| [k.to_s,v] }]",
                "Hash[_.map {...}]",
                1,
                3,
                1,
            ),
            (
                "é = 1; {}&.to_h { |k,v| [k.to_s,v] }",
                "to_h {...}",
                1,
                8,
                1,
            ),
        ] {
            let o: Vec<_> = lint_str(s)
                .into_iter()
                .filter(|o| o.cop == "Style/HashTransformKeys")
                .collect();
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col, o[0].end_line),
                (line, col, end),
                "{s}"
            );
            assert_eq!(
                o[0].message,
                format!("Prefer `transform_keys` over `{desc}`.")
            );
            assert!(o[0].correction.is_none());
        }
    }
}
