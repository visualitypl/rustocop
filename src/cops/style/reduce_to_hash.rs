//! `Style/ReduceToHash` — unsafe empty-hash accumulation advice (Ruby >=2.6).
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Range,
};
pub struct ReduceToHash;
impl Cop for ReduceToHash {
    fn name(&self) -> &'static str {
        "Style/ReduceToHash"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
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
        let Some(shape) = shape(&call) else { return };
        let facts = file.reduce_to_hash_facts(cx.ancestors().next_back().unwrap());
        if [&shape.key, &shape.value]
            .into_iter()
            .any(|n| facts.contains_read(n, shape.accumulator) || facts.contains_match(n))
        {
            return;
        }
        let selector = call.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `to_h {{ ... }}` instead of `{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(selector),
            correction: None,
        });
    }
}
struct Shape<'pr> {
    accumulator: &'pr [u8],
    key: Node<'pr>,
    value: Node<'pr>,
}
fn local(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|n| n.name().as_slice() == name)
}
// Constant-size structural predicate, also used when indexing nested *raw*
// matches. Deliberately lexical, like upstream: shadowed same-name reads count.
fn shape<'pr>(call: &CallNode<'pr>) -> Option<Shape<'pr>> {
    let each = match call.name().as_slice() {
        b"each_with_object" => true,
        b"inject" | b"reduce" => false,
        _ => return None,
    };
    let args = ast::call_arguments(call);
    let [initial] = args.as_slice() else {
        return None;
    };
    if !initial
        .as_hash_node()
        .is_some_and(|h| h.elements().is_empty())
    {
        return None;
    }
    let block = call.block()?.as_block_node()?;
    let parameters = block.parameters()?;
    let accumulator = if let Some(p) = parameters.as_numbered_parameters_node() {
        if p.maximum() != 2 {
            return None;
        }
        if each {
            b"_2".as_slice()
        } else {
            b"_1".as_slice()
        }
    } else {
        let p = parameters.as_block_parameters_node()?;
        if !p.locals().is_empty() {
            return None;
        }
        let p = p.parameters()?;
        if p.requireds().len() != 2
            || !p.optionals().is_empty()
            || p.rest()
                .is_some_and(|r| r.as_implicit_rest_node().is_none())
            || !p.posts().is_empty()
            || !p.keywords().is_empty()
            || p.keyword_rest().is_some()
            || p.block().is_some()
        {
            return None;
        }
        let mut names = p
            .requireds()
            .iter()
            .map(|n| n.as_required_parameter_node().map(|n| n.name().as_slice()));
        let first = names.next()??;
        let second = names.next()??;
        if each { second } else { first }
    };
    let body = block.body()?.as_statements_node()?;
    if body.body().len() != if each { 1 } else { 2 } {
        return None;
    }
    let mut statements = body.body().iter();
    let assign = statements.next()?.as_call_node()?;
    if !each && !local(&statements.next()?, accumulator) {
        return None;
    }
    if assign.is_safe_navigation()
        || assign.name().as_slice() != b"[]="
        || assign.block().is_some()
        || !assign.receiver().is_some_and(|r| local(&r, accumulator))
    {
        return None;
    }
    let args = ast::call_arguments(&assign);
    let Ok([key, value]): Result<[_; 2], _> = args.try_into() else {
        return None;
    };
    Some(Shape {
        accumulator,
        key,
        value,
    })
}
type Key = (usize, usize, u64);
fn key(n: &Node<'_>) -> Key {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::mem::discriminant(n).hash(&mut h);
    (
        n.location().start_offset(),
        n.location().end_offset(),
        h.finish(),
    )
}
/// One lazy main-AST walk. Preorder intervals, not byte containment, include
/// deferred heredoc descendants. Sorted lexical postings give O(log N) queries.
#[derive(Default)]
pub(crate) struct Facts {
    ranges: HashMap<Key, Range<usize>>,
    reads: HashMap<Vec<u8>, Vec<usize>>,
    matches: Vec<usize>,
    pub(crate) visits: usize,
}
impl Facts {
    fn contains_read(&self, node: &Node<'_>, name: &[u8]) -> bool {
        self.reads.get(name).is_some_and(|p| self.contains(node, p))
    }
    fn contains_match(&self, node: &Node<'_>) -> bool {
        self.contains(node, &self.matches)
    }
    fn contains(&self, node: &Node<'_>, positions: &[usize]) -> bool {
        let range = &self.ranges[&key(node)];
        positions
            .get(positions.partition_point(|p| *p < range.start))
            .is_some_and(|p| *p < range.end)
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Builder {
            facts: Facts,
            stack: Vec<(Key, usize)>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                let pos = self.facts.visits;
                self.facts.visits += 1;
                self.stack.push((key(&node), pos));
                if let Some(r) = node.as_local_variable_read_node() {
                    self.facts
                        .reads
                        .entry(r.name().as_slice().to_vec())
                        .or_default()
                        .push(pos);
                }
                if node.as_call_node().is_some_and(|c| shape(&c).is_some()) {
                    self.facts.matches.push(pos);
                }
            }
            fn leave(&mut self) {
                let (k, start) = self.stack.pop().unwrap();
                self.facts.ranges.insert(k, start..self.facts.visits);
            }
        }
        let mut b = Builder::default();
        walk(&mut b, root);
        b.facts
    }
}

pub(crate) static REDUCE_TO_HASH_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn reduce_to_hash_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        REDUCE_TO_HASH_FACTS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop reduce_to_hash_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/ReduceToHash")
            .collect()
    }
    #[test]
    fn explicit_and_numbered_accumulators_locations_and_messages() {
        for method in ["each_with_object", "inject", "reduce"] {
            let each = method == "each_with_object";
            for body in [
                if each {
                    "{ |elem,hash| hash[elem.id] = elem.name }"
                } else {
                    "{ |hash,elem| hash[elem.id] = elem.name; hash }"
                },
                if each {
                    "{ _2[_1.id] = _1.name }"
                } else {
                    "{ _1[_2.id] = _2.name; _1 }"
                },
                if each {
                    "do |elem,hash|\n hash[elem] = true\nend"
                } else {
                    "do |hash,elem|\n hash[elem] = true\n hash\nend"
                },
            ] {
                for (receiver, col) in [("array.", 7), ("array&.", 8), ("", 1)] {
                    let s = format!("# comment\n{receiver}{method}({{}}) {body}");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (2, col, 2)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Use `to_h {{ ... }}` instead of `{method}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn trailing_comma_is_not_a_rest_parameter() {
        assert_eq!(
            offenses("xs.each_with_object({}) { |e,h,| h[e]=e }").len(),
            1
        );
        assert_eq!(offenses("xs.reduce({}) { |h,e,| h[e]=e; h }").len(), 1);
        assert!(offenses("xs.each_with_object({}) { |e,h,*rest| h[e]=e }").is_empty());
    }
    #[test]
    fn arbitrary_key_value_expressions_include_command_assignment_splat_yield_and_rescue() {
        for (key, value) in [
            ("elem.id", "do_something elem"),
            ("obj.do_something elem", "elem.name"),
            ("elem.id", "yield elem"),
            ("elem.id", "elem.name, elem.value"),
            ("elem.id", "*elem"),
            ("elem.id", "x = do_something elem"),
            ("elem.id", "do_something(elem) rescue nil"),
            ("elem.id + 1", "do_something(elem)"),
            ("elem.id", "begin; next 3 if elem; elem; end"),
            ("elem.id", "-> { break 1 }"),
        ] {
            let s = format!(
                "def example; array.each_with_object({{}}) {{ |elem,hash| hash[{key}] = {value} }}; end"
            );
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
    }
    #[test]
    fn rejects_nonmatching_bodies_arity_initializers_and_accumulator_references() {
        for s in [
            "array.each_with_object({}) { |elem,hash| hash[elem]=true; puts elem }",
            "array.each_with_object({}) { |(k,v),h| h[k.to_s]=v }",
            "array.each_with_object(Hash.new(0)) { |e,h| h[e]+=1 }",
            "array.each_with_object({a: 1}) { |e,h| h[e]=1 }",
            "array.each_with_object({}, 1) { |e,h| h[e]=1 }",
            "array.each_with_object({}) { |e,h| h.merge!(e=>true) }",
            "array.each_with_object({}) { |e,h| other[e]=1 }",
            "array.each_with_object({}) { |e,h| h[h.size]=e }",
            "array.each_with_object({}) { |e,h| h[e]=h[e].to_i+1 }",
            "array.each_with_object({}) { |e,h| h[e]=->(h) { h } }",
            "array.each_with_object({}) { |e,h; local| h[e]=e }",
            "array.each_with_object({}) { |e,h=nil| h[e]=e }",
            "array.each_with_object({}) { |e,*h| h[e]=e }",
            "array.each_with_object({}) { |e,h,x| h[e]=e }",
            "array.each_with_object({}) { |e,h| h[e,1]=e }",
            "array.each_with_object({}) { |e,h| h[e]+=e }",
            "array.each_with_object({}) { _2[_1]=_2[_1].to_i+1 }",
            "array.each_with_object({}) { _3[_1]=_2 }",
            "array.each_with_object({}) { it[it]=1 }",
            "array.inject({}) { |h,e| h[e]=true; other }",
            "array.inject({}) { |h,e| puts e; h[e]=true; h }",
            "array.inject({}) { |h,e| h[e]=h[e].to_i+1; h }",
            "array.reduce({}) { |h,e| h[e]=e }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn nested_raw_matches_suppress_outer_but_not_inner_even_when_inner_is_exempt() {
        let s = "tables.each_with_object({}) { |table,h|\n h[table.node] = table.columns.each_with_object({}) { |column,i| i[column.name] = column.alias }\n}";
        let found = offenses(s);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 32));
        let s = "xs.each_with_object({}) { |x,h| h[x]=ys.reduce({}) { |a,y| a[y]=a.size; a } }";
        assert!(offenses(s).is_empty());
        // Symbols and writes alone are not reads. Upstream uses lexical lvar
        // names even in shadowed scopes; do not substitute binding-sensitive uses.
        assert_eq!(
            offenses("xs.each_with_object({}) { |x,h| h[x]=:h }").len(),
            1
        );
        assert_eq!(
            offenses("xs.each_with_object({}) { |x,h| h[x]=(h=1) }").len(),
            1
        );
    }
}
