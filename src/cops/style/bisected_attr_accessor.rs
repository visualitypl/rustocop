//! `Style/BisectedAttrAccessor` — combine matching readers and writers.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct BisectedAttrAccessor;
impl Cop for BisectedAttrAccessor {
    fn name(&self) -> &'static str {
        "Style/BisectedAttrAccessor"
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
        let Some(children) = super::accessors::children(node) else {
            return;
        };
        let mut macros = Vec::new();
        let mut names = std::collections::HashMap::new();
        let mut visibility = b"public".to_vec();
        for (i, child) in children.iter().enumerate() {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            if let Some(v) = super::accessors::visibility(&call) {
                visibility = v.to_vec();
            }
            if call.is_safe_navigation() || super::accessors::literal_block(&call) {
                continue;
            }
            let bit = match call.name().as_slice() {
                b"attr_reader" | b"attr" => 1u8,
                b"attr_writer" => 2,
                _ => continue,
            };
            let args = crate::cops::ast::call_arguments(&call);
            // Upstream keys names by source, not decoded symbol value. Repeated
            // source keys retain only their last argument's offense position.
            let mut last = std::collections::HashMap::new();
            for (n, arg) in args.iter().enumerate() {
                let text = arg.location().as_slice().to_vec();
                *names
                    .entry((visibility.clone(), text.clone()))
                    .or_insert(0u8) |= bit;
                last.insert(text, n);
            }
            macros.push((i, visibility.clone(), bit, args, last));
        }
        // `find_bisection`: per visibility, reader names that writers share,
        // in reader order.
        let mut bisected: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (_, visibility, bit, args, _) in &macros {
            if *bit != 1 {
                continue;
            }
            for arg in args {
                let key = (visibility.clone(), arg.location().as_slice().to_vec());
                if names.get(&key) == Some(&3) && !bisected.contains(&key) {
                    bisected.push(key);
                }
            }
        }
        for (i, visibility, bit, args, last) in macros {
            let correction = cx
                .corrections_requested()
                .then(|| correction(&children[i], bit, &args, &visibility, &bisected, file))
                .flatten();
            let mut correction = correction;
            for (n, arg) in args.iter().enumerate() {
                let text = arg.location().as_slice();
                if last.get(text) != Some(&n)
                    || names.get(&(visibility.clone(), text.to_vec())) != Some(&3)
                {
                    continue;
                }
                let (line, col) = file.line_col(arg.location().start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Combine both accessors into `attr_accessor {}`.",
                        String::from_utf8_lossy(text)
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(arg.location()),
                    correction: correction.take(),
                });
            }
        }
    }
}
/// RuboCop's `after_class` rewrite of one macro.
fn correction(
    node: &Node<'_>,
    bit: u8,
    args: &[Node<'_>],
    visibility: &[u8],
    bisected: &[(Vec<u8>, Vec<u8>)],
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    use crate::offense::Correction;
    let mut keys: Vec<&[u8]> = Vec::new();
    for arg in args {
        let text = arg.location().as_slice();
        if !keys.contains(&text) {
            keys.push(text);
        }
    }
    let shared: Vec<&[u8]> = bisected
        .iter()
        .filter(|(v, n)| v == visibility && keys.contains(&n.as_slice()))
        .map(|(_, n)| n.as_slice())
        .collect();
    if shared.is_empty() {
        return None;
    }
    let join = |names: &[&[u8]]| {
        names
            .iter()
            .map(|n| std::str::from_utf8(n).ok())
            .collect::<Option<Vec<_>>>()
            .map(|n| n.join(", "))
    };
    let rest: Vec<&[u8]> = keys.into_iter().filter(|k| !shared.contains(k)).collect();
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    let lines = super::correction::whole_lines(file, range.clone());
    let line_start = file.line_start(file.line_of(loc.start_offset()));
    let indent = " ".repeat(
        std::str::from_utf8(&file.bytes[line_start..loc.start_offset()])
            .ok()?
            .chars()
            .count(),
    );
    let edit = |range, replacement| Correction { range, replacement };
    if bit == 1 {
        let accessor = format!("attr_accessor {}\n", join(&shared)?);
        if rest.is_empty() {
            return Some(edit(lines, format!("{indent}{accessor}")));
        }
        let at = range.start;
        return super::correction::combine(
            file,
            vec![
                edit(at..at, accessor),
                edit(range, format!("{indent}attr_reader {}", join(&rest)?)),
            ],
        );
    }
    Some(if rest.is_empty() {
        edit(lines, String::new())
    } else {
        edit(range, format!("attr_writer {}", join(&rest)?))
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/bisected_attr_accessor_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/BisectedAttrAccessor")
            .collect()
    }
    #[test]
    fn exact_argument_positions_and_message() {
        for reader in ["attr_reader", "attr"] {
            let s = format!("class Foo\n  {reader} :bar\n  attr_writer :bar\nend\n");
            let found = offenses(&s);
            assert_eq!(found.len(), 2, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, reader.len() as u32 + 4));
            assert_eq!((found[1].line, found[1].col), (3, 15));
            assert!(
                found
                    .iter()
                    .all(|o| o.message == "Combine both accessors into `attr_accessor :bar`.")
            );
        }
    }
    #[test]
    fn splats_partial_families_and_reentered_visibility() {
        for (body, count) in [
            ("attr_reader *ATTRIBUTES\nattr_writer *ATTRIBUTES", 2),
            ("attr_reader :baz, :bar, :quux\nattr_writer :bar, :zoo", 2),
            ("attr_reader :foo, :bar, :baz\nattr_writer :foo, :baz", 4),
            ("attr_writer :foo\nattr_reader :foo", 2),
            (
                "attr_reader :foo\nprivate\nattr_writer :bar\npublic\nattr_writer :foo",
                2,
            ),
        ] {
            assert_eq!(
                offenses(&format!("module Foo\n{body}\nend")).len(),
                count,
                "{body}"
            );
        }
    }
    #[test]
    fn different_visibility_scopes_and_indirect_bodies_are_not_combined() {
        for body in [
            "attr_reader :bar\nprivate\nattr_writer :bar",
            "attr_reader :bar\nattr_writer :baz",
            "attr_accessor :bar",
            "attr_reader :bar\ndef f; attr_writer :bar; end",
            "attr_reader :bar\nclass << self; attr_writer :bar; end",
        ] {
            assert!(
                offenses(&format!("class Foo\n{body}\nend")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn receivers_blocks_dynamic_sources_and_duplicate_argument_locations() {
        let s = "class Foo\n  Other.attr_reader :a\n  self.attr_writer :a\n  attr_reader :b, :b\n  attr_writer :b\n  attr_reader :\"a\"\n  attr_writer :a\nend\n";
        let found = offenses(s);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 21), (3, 20), (4, 19), (5, 15), (7, 15)]
        );
        for s in [
            "class Foo; attr_reader(:a) {}; attr_writer :a; end",
            "class Foo; self&.attr_reader :a; attr_writer :a; end",
            "class Foo; attr_reader :a; attr_writer :\"a\"; end",
            "class Foo; private attr_reader :a; attr_writer :a; end",
            "attr_reader :a\nattr_writer :a",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_module_visibility_and_eigenclass_canonical_pairs() {
        for body in [
            "  attr_reader :a\n  attr_writer :a\n\n  private\n  attr_reader :b\n  attr_writer :b",
            "  class << self\n    attr_reader :baz\n    attr_writer :baz\n    private\n    attr_reader :quux\n  end",
        ] {
            let after = body
                .replace("attr_reader :a\n  attr_writer :a", "attr_accessor :a")
                .replace("attr_reader :b\n  attr_writer :b", "attr_accessor :b")
                .replace(
                    "attr_reader :baz\n    attr_writer :baz",
                    "attr_accessor :baz",
                );
            for kind in ["class", "module"] {
                crate::cops::testing::assert_correction(
                    &super::BisectedAttrAccessor,
                    &format!("{kind} Foo\n{body}\nend\n"),
                    &format!("{kind} Foo\n{after}\nend\n"),
                );
            }
        }
    }
    #[test]
    fn explicit_begin_body_keeps_argument_locations() {
        let found = offenses("class Foo\nbegin\nattr_reader :a\nattr_writer :a\nend\nend");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 13), (4, 13)]
        );
    }
    #[test]
    fn canonical_adjacent_complete_pairs_correct_atomically() {
        for reader in ["attr_reader", "attr"] {
            for names in [":bar", ":foo, :bar, :baz"] {
                crate::cops::testing::assert_correction(
                    &super::BisectedAttrAccessor,
                    &format!("class Foo\n  {reader} {names}\n  attr_writer {names}\nend\n"),
                    &format!("class Foo\n  attr_accessor {names}\nend\n"),
                );
            }
        }
    }
}
