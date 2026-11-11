//! `Bundler/DuplicatedGroup`: a gem group, or set of groups, declared more
//! than once in a supplied Gemfile under the same nearest `source`, `git`,
//! `platforms` or `path` block (the pinned string key, quirks included).
use std::collections::HashMap;

use ruby_prism::{CallNode, Node};

use super::declarations::{receiverless_calls, source_bytes, str_value, text};
use crate::{
    cops::{Cop, ast, lint::reference_basic_literal::is_basic_literal},
    offense::Offense,
    source::SourceFile,
};

pub struct DuplicatedGroup;

/// `SOURCE_BLOCK_NAMES`.
const SOURCE_BLOCKS: [&[u8]; 4] = [b"source", b"git", b"platforms", b"path"];

struct Declaration {
    start: usize,
    key: Vec<u8>,
    name: Vec<u8>,
}

impl Cop for DuplicatedGroup {
    fn name(&self) -> &'static str {
        "Bundler/DuplicatedGroup"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let declarations = receiverless_calls(&result.node(), b"group", |call, ancestors| {
            let arguments = arguments(call);
            let mut key = source_key(ancestors, file);
            let mut attributes: Vec<_> = arguments.iter().map(|a| attribute(a, file)).collect();
            attributes.sort();
            attributes.into_iter().for_each(|a| key.extend(a));
            let name = arguments
                .iter()
                .map(|a| source(a, file))
                .collect::<Vec<_>>()
                .join(&b", "[..]);
            Some(Declaration {
                start: call.location().start_offset(),
                key,
                name,
            })
        });
        let mut first: HashMap<&[u8], usize> = HashMap::new();
        let mut reported: Vec<(usize, usize)> = Vec::new();
        for (i, declaration) in declarations.iter().enumerate() {
            match first.get(declaration.key.as_slice()) {
                Some(&earlier) => reported.push((i, earlier)),
                None => {
                    first.insert(&declaration.key, i);
                }
            }
        }
        for (i, earlier) in reported {
            let declaration = &declarations[i];
            let (first_line, _) =
                crate::cops::lint::reference_line_col(declarations[earlier].start, file);
            let (line, col) = crate::cops::lint::reference_line_col(declaration.start, file);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: line,
                message: format!(
                    "Gem group `{}` already defined on line {first_line} of the Gemfile.",
                    text(&declaration.name)
                ),
                correction: None,
            });
        }
    }
}

/// The pinned `send` arguments: a block-pass included, a literal block not.
fn arguments<'pr>(call: &CallNode<'pr>) -> Vec<Node<'pr>> {
    let mut list: Vec<_> = call
        .arguments()
        .iter()
        .flat_map(|a| a.arguments().iter())
        .collect();
    list.extend(
        call.block()
            .filter(|b| b.as_block_argument_node().is_some()),
    );
    list
}

fn source(node: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    let location = node.location();
    source_bytes(&file.bytes[location.start_offset()..location.end_offset()])
}

/// `find_source_key`: the nearest enclosing `block` (not numblock/itblock)
/// sent `source`/`git`/`platforms`/`path`, with its first argument's source.
fn source_key(ancestors: &[Node<'_>], file: &SourceFile) -> Vec<u8> {
    for ancestor in ancestors.iter().rev() {
        let Some(call) = ancestor.as_call_node() else {
            continue;
        };
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            continue;
        };
        if block.parameters().is_some_and(|p| {
            p.as_numbered_parameters_node().is_some() || p.as_it_parameters_node().is_some()
        }) {
            continue;
        }
        let name = call.name().as_slice();
        if SOURCE_BLOCKS.contains(&name) {
            let mut key = name.to_vec();
            if let Some(first) = arguments(&call).first() {
                key.extend(source(first, file));
            }
            return key;
        }
    }
    Vec::new()
}

/// `group_attributes` for one argument: sorted pair sources of a hash, the
/// `value.to_s` of a node with a value, and the source of anything else.
fn attribute(node: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    let pairs = node
        .as_keyword_hash_node()
        .map(|h| h.elements())
        .or_else(|| node.as_hash_node().map(|h| h.elements()));
    if let Some(elements) = pairs {
        let mut pairs: Vec<_> = elements
            .iter()
            .filter(|e| e.as_assoc_node().is_some())
            .map(|e| source(&e, file))
            .collect();
        pairs.sort();
        return pairs.join(&b", "[..]);
    }
    value(node, file).unwrap_or_else(|| source(node, file))
}

/// `value.to_s` for the nodes RuboCop's AST gives a `value`: str/dstr, sym
/// and integers. Other numeric literals and xstr keep their source here.
fn value(node: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    match node {
        Node::StringNode { .. } => Some(node.as_string_node()?.unescaped().to_vec()),
        Node::InterpolatedStringNode { .. } => {
            if let Some(value) = str_value(node, file) {
                return Some(value);
            }
            // DstrNode#value: child values, or the source of a child without one.
            let mut value = Vec::new();
            for part in node.as_interpolated_string_node()?.parts().iter() {
                match &part {
                    Node::StringNode { .. } | Node::InterpolatedStringNode { .. } => {
                        value.extend(self::value(&part, file)?);
                    }
                    Node::EmbeddedVariableNode { .. } => {
                        value.extend(source(&part.as_embedded_variable_node()?.variable(), file));
                    }
                    _ => value.extend(source(&part, file)),
                }
            }
            Some(value)
        }
        Node::SymbolNode { .. } if is_basic_literal(node, file) => {
            Some(node.as_symbol_node()?.unescaped().to_vec())
        }
        Node::IntegerNode { .. } => {
            let integer = node.as_integer_node()?.value();
            let (negative, digits) = integer.to_u32_digits();
            let mut value = ast::unsigned_integer_decimal(digits);
            if negative && value != "0" {
                value.insert(0, '-');
            }
            Some(value.into_bytes())
        }
        Node::SourceLineNode { .. } => {
            let (line, _) =
                crate::cops::lint::reference_line_col(node.location().start_offset(), file);
            Some(line.to_string().into_bytes())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's bundler/duplicated_group_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov), plus lookalikes.
    use super::*;
    use crate::cops::bundler::tests::{assert_offenses, lint};

    fn expect(source: &str, expected: &[(u32, u32, &str, u32)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(line, col, group, first)| {
                (
                    line,
                    col,
                    format!("Gem group `{group}` already defined on line {first} of the Gemfile."),
                )
            })
            .collect();
        assert_offenses(&DuplicatedGroup, "Gemfile", source, &expected);
    }

    #[test]
    fn ruby_files_and_empty_gemfiles_are_ignored() {
        let source = "# cop will not read these contents\ngroup :development\ngroup :development\n";
        assert!(lint(&DuplicatedGroup, "foo.rb", source).is_empty());
        expect("", &[]);
    }

    #[test]
    fn distinct_groups() {
        expect(
            "group :development do\n  gem 'rubocop'\nend\ngroup :test do\n  gem 'flog'\nend\n",
            &[],
        );
        expect(
            "group :development do\n  gem 'rubocop'\nend\ngroup :development, :test do\n  gem 'rspec'\nend\ngroup :ci, :development do\n  gem 'flog'\nend\n",
            &[],
        );
    }

    #[test]
    fn duplicated_group_and_argument_types() {
        expect(
            "group :development do\n  gem 'rubocop'\nend\ngroup :development do\n  gem 'rubocop-rails'\nend\n",
            &[(4, 1, ":development", 1)],
        );
        expect(
            "group :development do\n  gem 'rubocop'\nend\ngroup 'development' do\n  gem 'rubocop-rails'\nend\n",
            &[(4, 1, "'development'", 1)],
        );
    }

    #[test]
    fn keyword_options() {
        for second in ["bar: true", "foo: false"] {
            expect(
                &format!(
                    "group :test, foo: true do\n  gem 'activesupport'\nend\n\ngroup :test, {second} do\n  gem 'rspec'\nend\n"
                ),
                &[],
            );
        }
        for second in ["foo: true, bar: true", "bar: true, foo: true"] {
            expect(
                &format!(
                    "group :test, foo: true, bar: true do\n  gem 'activesupport'\nend\n\ngroup :test, {second} do\n  gem 'rspec'\nend\n"
                ),
                &[(5, 1, &format!(":test, {second}"), 1)],
            );
        }
    }

    #[test]
    fn group_sets_and_splats() {
        expect(
            "group :test, :development do\n  gem 'rubocop'\nend\ngroup :development, :test do\n  gem 'rubocop-rails'\nend\n",
            &[(4, 1, ":development, :test", 1)],
        );
        expect(
            "group(*LIVE_ENVS) do\n  gem 'admin_ui'\nend\n\ngroup(*LIVE_ENVS) do\n  gem 'public_ui'\nend\n",
            &[(5, 1, "*LIVE_ENVS", 1)],
        );
    }

    #[test]
    fn source_blocks() {
        expect(
            "source 'https://rubygems.pkg.github.com/private-org' do\n  group :development do\n    gem 'rubocop'\n  end\nend\n\ngroup :development do\n  gem 'rubocop-rails'\nend\n",
            &[],
        );
        expect(
            "source 'https://rubygems.pkg.github.com/private-org' do\n  group :development do\n    gem 'rubocop'\n  end\nend\n\nsource 'https://rubygems.pkg.github.com/private-org' do\n  group :development do\n    gem 'rubocop-rails'\n  end\nend\n\ngroup :development do\n  gem 'rubocop-performance'\nend\n",
            &[(8, 3, ":development", 2)],
        );
        for (block, a, b) in [
            (
                "git",
                "'https://github.com/rubocop/rubocop.git'",
                "'https://github.com/rails/rails.git'",
            ),
            ("platforms", ":ruby", ":jruby"),
            ("path", "'components_admin'", "'components_public'"),
        ] {
            let source = |second: &str| {
                format!(
                    "{block} {a} do\n  group :default do\n    gem 'x'\n  end\nend\n\n{block} {second} do\n  group :default do\n    gem 'y'\n  end\nend\n"
                )
            };
            expect(&source(b), &[]);
            expect(&source(a), &[(8, 3, ":default", 2)]);
        }
        expect(
            "source do\n  group :development do\n    gem 'rubocop'\n  end\nend\n",
            &[],
        );
    }

    #[test]
    fn pinned_key_quirks() {
        // The attribute strings are joined without a separator.
        expect(
            "group :a, :b do\nend\ngroup :ab do\nend\n",
            &[(3, 1, ":ab", 1)],
        );
        expect("group\ngroup\n", &[(2, 1, "", 1)]);
        expect("foo.group :a\nfoo.group :a\n", &[]);
        expect(
            "group :a do\nend\ngroup 'a' do\nend\ngroup %s(a) do\nend\n",
            &[(3, 1, "'a'", 1), (5, 1, "%s(a)", 1)],
        );
        expect("group :\"#{x}\" do\nend\ngroup '#{x}' do\nend\n", &[]);
        expect(
            "group \"#{x}\" do\nend\ngroup '#{x}' do\nend\n",
            &[(3, 1, "'#{x}'", 1)],
        );
        expect(
            "group 1 do\nend\ngroup '1' do\nend\ngroup 0x1 do\nend\n",
            &[(3, 1, "'1'", 1), (5, 1, "0x1", 1)],
        );
        expect(
            "group :a, foo: 1 do\nend\ngroup :a, :foo => 1 do\nend\n",
            &[],
        );
        expect(
            "group :a, **opts do\nend\ngroup :a do\nend\n",
            &[(3, 1, ":a", 1)],
        );
        expect(
            "group(:a, &blk)\ngroup(:a, &blk)\n",
            &[(2, 1, ":a, &blk", 1)],
        );
        expect(
            "group :a, 'b' 'c'\ngroup :a, \"b#@c\"\ngroup :a, 'bc'\n",
            &[(3, 1, ":a, 'bc'", 1)],
        );
        expect(
            "group :a, 2\ngroup :a, __LINE__\n",
            &[(2, 1, ":a, __LINE__", 1)],
        );
    }

    #[test]
    fn nearest_plain_block_only() {
        // A numbered-parameter block is not a `block` node; an `it` block neither.
        expect(
            "source 'x' do\n  group :a do\n  end\nend\nsource 'x' do\n  _1\n  group :a do\n  end\nend\n",
            &[],
        );
        expect(
            "source 'x' do\n  it\n  group :a do\n  end\nend\ngroup :a do\nend\n",
            &[(6, 1, ":a", 3)],
        );
        expect(
            "source 'x' do\n  path 'y' do\n    group :a do\n    end\n  end\nend\npath 'y' do\n  group :a do\n  end\nend\n",
            &[(8, 3, ":a", 3)],
        );
        expect(
            "foo.source 'x' do\n  group :a do\n  end\nend\nsource 'x' do\n  group :a do\n  end\nend\n",
            &[(6, 3, ":a", 2)],
        );
        expect(
            "source('x', group(:a)) do\nend\nsource 'x' do\n  group :a\nend\n",
            &[(4, 3, ":a", 1)],
        );
        expect(
            "unless c\n  group :a\nelse\n  group :a, :b\n  group :b, :a\nend\n",
            &[(5, 3, ":b, :a", 4)],
        );
    }

    #[test]
    fn crlf_bom_and_multiline_names() {
        expect("group :a\r\ngroup :a\r\n", &[(2, 1, ":a", 1)]);
        expect("\u{feff}group :a\ngroup :a\n", &[(2, 1, ":a", 1)]);
        expect(
            "group :a, b: [\r\n  1\r\n]\r\ngroup :a, b: [\r\n  1\r\n]\r\n",
            &[(4, 1, ":a, b: [\n  1\n]", 1)],
        );
        expect("group(:café)\ngroup('café')\n", &[(2, 1, "'café'", 1)]);
    }
}
