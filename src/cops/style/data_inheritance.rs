//! `Style/DataInheritance` — Ruby >= 3.2, detection only (unsafe rewrite).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DataInheritance;
impl Cop for DataInheritance {
    fn name(&self) -> &'static str {
        "Style/DataInheritance"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        super::struct_inheritance::check_inheritance(
            self,
            node,
            cx,
            file,
            out,
            ("Data", b"define", "class"),
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop examples: see the MIT attribution in struct_inheritance::tests.
    use super::super::struct_inheritance::tests::{
        constant_exemptions, lookalikes, pinned_offenses,
    };
    use super::*;
    #[test]
    fn pinned_data_offenses_and_unsafe_corrections() {
        pinned_offenses(
            &DataInheritance,
            "Data.define",
            "Don't extend an instance initialized by `Data.define`. Use a block to customize the class.",
        );
    }
    #[test]
    fn data_constant_definitions_exempt_the_class() {
        constant_exemptions("Style/DataInheritance", "Data.define");
    }
    #[test]
    fn data_lookalikes_are_ignored() {
        lookalikes("Style/DataInheritance", "Data.define");
    }
}
