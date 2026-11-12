//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/redundant_line_break_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov).
//!
//! `PINNED`: all 123 pinned examples (the shared `common behavior` examples
//! under both `InspectBlocks` contexts, the block contexts, the shared-line
//! and `Layout/LineLength`-disabled contexts) replayed at the fixed defaults
//! (`InspectBlocks: false`, `Layout/LineLength` `max` 120,
//! `Layout/SingleLineBlockChain` disabled). Findings and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported), except the
//! two rows of spec line 367, whose findings are kept without upstream's
//! `"a" + 'b'` edit (`HELD`).
//!
//! `SPEC`: every expectation of the examples whose configuration is
//! representable (all but the `InspectBlocks: true` contexts, which are not
//! applicable at the fixed `false`), at the spec's own `Layout/LineLength`
//! `max` (or disabled) and `Layout/SingleLineBlockChain` enablement, with
//! the spec's annotated positions and `expect_correction` bytes.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/RedundantLineBreak";
const TRUE: &str = "InspectBlocks: true context, replayed at the fixed false";
const FALSE: &str = "InspectBlocks: false context";
const MAIN: &str = "top-level context";
/// The finding is kept without an edit where upstream turns a split
/// string into a `+` call.
const HELD: &str = "HELD";

/// `(context, spec line, source, findings, single-pass correction)`; an
/// empty correction means the source has no findings.
type Pinned = (
    &'static str,
    u32,
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (
        TRUE,
        24,
        "e.select { |i| i.cond? }\n  .join\na = e.select { |i| i.cond? }\n  .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }.join\na = e.select { |i| i.cond? }.join\ne.select { |i| i.cond? }.join + []\n",
    ),
    (
        TRUE,
        44,
        "e&.select { |i| i.cond? }\n  &.join\na = e&.select { |i| i.cond? }\n  &.join\ne&.select { |i| i.cond? }\n  &.join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e&.select { |i| i.cond? }&.join\na = e&.select { |i| i.cond? }&.join\ne&.select { |i| i.cond? }&.join + []\n",
    ),
    (
        TRUE,
        69,
        "e.select { |i| i.cond? }\n .join\na = e.select { |i| i.cond? }\n     .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }.join\na = e.select { |i| i.cond? }.join\ne.select { |i| i.cond? }.join + []\n",
    ),
    (
        TRUE,
        80,
        "e.select { |i| i.cond? }\n &.join\na = e.select { |i| i.cond? }\n     &.join\ne.select { |i| i.cond? }\n &.join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }&.join\na = e.select { |i| i.cond? }&.join\ne.select { |i| i.cond? }&.join + []\n",
    ),
    (
        TRUE,
        91,
        "e.select { _1.cond? }\n .join\na = e.select { _1.cond? }\n .join\ne.select { _1.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { _1.cond? }.join\na = e.select { _1.cond? }.join\ne.select { _1.cond? }.join + []\n",
    ),
    (
        TRUE,
        102,
        "e.select { it.cond? }\n .join\na = e.select { it.cond? }\n .join\ne.select { it.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { it.cond? }.join\na = e.select { it.cond? }.join\ne.select { it.cond? }.join + []\n",
    ),
    (
        TRUE,
        115,
        "a =\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        TRUE,
        126,
        "a =\n  case x\n  when :a\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        TRUE,
        138,
        "a +\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (TRUE, 149, "a do\n  x\n  y\nend\n", &[], ""),
    (TRUE, 158, "a ||= begin\n  x\n  y\nend\n", &[], ""),
    (TRUE, 167, "x def self.y\n    z\n  end\n", &[], ""),
    (TRUE, 175, "my_method(1, 2, \"x\")\n", &[], ""),
    (
        TRUE,
        181,
        "my_method(1) \\\n  [:a]\n",
        &[(1, 1)],
        "my_method(1) [:a]\n",
    ),
    (TRUE, 193, "hash[:foo] \\\n  [:bar]\n", &[], ""),
    (
        TRUE,
        200,
        "{\n  key: value\n}[key]\n",
        &[(1, 1)],
        "{ key: value }[key]\n",
    ),
    (TRUE, 213, "foo && \\\n  bar\n", &[(1, 1)], "foo && bar\n"),
    (TRUE, 225, "foo \\\n  && bar\n", &[], ""),
    (TRUE, 232, "foo || \\\n  bar\n", &[(1, 1)], "foo || bar\n"),
    (TRUE, 244, "foo \\\n  || bar\n", &[], ""),
    (
        TRUE,
        293,
        "my_method(1,\n          2,\n          \"x\")\n",
        &[(1, 1)],
        "my_method(1, 2, \"x\")\n",
    ),
    (
        TRUE,
        306,
        "some_array.map do |something|\n  my_method(\n    something,\n  )\nend\n",
        &[(2, 3)],
        "some_array.map do |something|\n  my_method( something, )\nend\n",
    ),
    (
        TRUE,
        323,
        "my_method(1,\n          2,\n          \"x\") # X\n",
        &[],
        "",
    ),
    (
        TRUE,
        331,
        "my_method(\"a\" \\\n          \"b\")\n",
        &[(1, 1)],
        "my_method(\"ab\")\n",
    ),
    (
        TRUE,
        343,
        "puts \"(#{pl(i)}, \" \\\n     \"#{pl(f)})\"\n",
        &[(1, 1)],
        "puts \"(#{pl(i)}, #{pl(f)})\"\n",
    ),
    (
        TRUE,
        355,
        "my_method('a'\\\n          'b')\n",
        &[(1, 1)],
        "my_method('ab')\n",
    ),
    (
        TRUE,
        367,
        "my_method(\"a\" \\\n          'b')\nmy_method('a' \\\n          \"b\")\n",
        &[(1, 1), (3, 1)],
        HELD,
    ),
    (
        TRUE,
        383,
        "my_method(1 +\n          2 +\n          3)\n",
        &[(1, 1)],
        "my_method(1 + 2 + 3)\n",
    ),
    (
        TRUE,
        396,
        "a =\n  m(1 +\n    2 +\n    3)\nb = m(4 +\n      5 +\n      6)\nlong_variable_name =\n  m(7 +\n    8 +\n    9)\n",
        &[(1, 1), (5, 1), (8, 1)],
        "a = m(1 + 2 + 3)\nb = m(4 + 5 + 6)\nlong_variable_name = m(7 + 8 + 9)\n",
    ),
    (
        TRUE,
        422,
        "a = b ||= {\n  x: 1\n}\n",
        &[(1, 1)],
        "a = b ||= { x: 1 }\n",
    ),
    (
        TRUE,
        254,
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError,\n            \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
        &[(5, 7)],
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError, \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
    ),
    (
        TRUE,
        436,
        "foo(' .x')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(' .x').bar.baz\n",
    ),
    (
        TRUE,
        449,
        "foo(x,\n    y,\n    z)\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(x, y, z).bar.baz\n",
    ),
    (
        TRUE,
        464,
        "foo('....' \\\n    '....')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo('........').bar.baz\n",
    ),
    (
        TRUE,
        478,
        "foo(<<~EOS)\n  xyz\nEOS\n  .bar\n  .baz\n",
        &[],
        "",
    ),
    (TRUE, 488, "foo('\n  xyz\n')\n  .bar\n  .baz\n", &[], ""),
    (TRUE, 498, "%\n\n", &[], ""),
    (TRUE, 502, "x = %\n\n", &[], ""),
    (
        TRUE,
        509,
        "my_method(11111,\n          22222,\n          \"abcxyz\")\nmy_method(111111 +\n          222222 +\n          333333)\n",
        &[(1, 1), (4, 1)],
        "my_method(11111, 22222, \"abcxyz\")\nmy_method(111111 + 222222 + 333333)\n",
    ),
    (TRUE, 520, "foo(:\"\n\")\n", &[], ""),
    (
        TRUE,
        530,
        "VariableReference = Struct.new(:name) do\n  def assignment?\n    false\n  end\nend\n",
        &[],
        "",
    ),
    (
        TRUE,
        540,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) && a && File.basename(path).start_with?('.') && !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        TRUE,
        546,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) + a + File.basename(path).start_with?('.') + !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        TRUE,
        555,
        "correct = lambda do\n  expect_no_offenses(<<~EOT1)\n    <<-EOT2\n    foo\n    EOT2\n  EOT1\nend\n",
        &[],
        "",
    ),
    (
        TRUE,
        567,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { |n| n.loc.heredoc_body }\n    .flat_map { |b| (b.line...b.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        TRUE,
        576,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { _1.loc.heredoc_body }\n    .flat_map {  (_1.line..._1.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        TRUE,
        585,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345678' do\nend\n",
        &[(1, 1)],
        "context 'when the configuration includes an unsafe cop that is 123456789012345678' do\nend\n",
    ),
    (
        TRUE,
        593,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345' do\nend\n",
        &[(1, 1)],
        "context 'when the configuration includes an unsafe cop that is 123456789012345' do\nend\n",
    ),
    (
        TRUE,
        608,
        "RSpec.shared_context 'ruby 2.4', :ruby24 do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        TRUE,
        616,
        "do_something do\nrescue CustomError\nend\n",
        &[],
        "",
    ),
    (TRUE, 624, "do_something do\nensure\nend\n", &[], ""),
    (
        TRUE,
        644,
        "RSpec.shared_context('ruby 2.4', :ruby24) do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        TRUE,
        657,
        "RSpec.shared_context do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (TRUE, 673, "f do\nend\n", &[], ""),
    (TRUE, 689, "f do\nend\n", &[], ""),
    (TRUE, 701, "e.select do |i|\n  i.cond?\nend.join\n", &[], ""),
    (
        TRUE,
        712,
        "a = e.select do |i|\n  i.cond?\nend.join\n",
        &[],
        "",
    ),
    (
        TRUE,
        723,
        "e.select do |i|\n  i.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        FALSE,
        24,
        "e.select { |i| i.cond? }\n  .join\na = e.select { |i| i.cond? }\n  .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }.join\na = e.select { |i| i.cond? }.join\ne.select { |i| i.cond? }.join + []\n",
    ),
    (
        FALSE,
        44,
        "e&.select { |i| i.cond? }\n  &.join\na = e&.select { |i| i.cond? }\n  &.join\ne&.select { |i| i.cond? }\n  &.join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e&.select { |i| i.cond? }&.join\na = e&.select { |i| i.cond? }&.join\ne&.select { |i| i.cond? }&.join + []\n",
    ),
    (
        FALSE,
        69,
        "e.select { |i| i.cond? }\n .join\na = e.select { |i| i.cond? }\n     .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }.join\na = e.select { |i| i.cond? }.join\ne.select { |i| i.cond? }.join + []\n",
    ),
    (
        FALSE,
        80,
        "e.select { |i| i.cond? }\n &.join\na = e.select { |i| i.cond? }\n     &.join\ne.select { |i| i.cond? }\n &.join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }&.join\na = e.select { |i| i.cond? }&.join\ne.select { |i| i.cond? }&.join + []\n",
    ),
    (
        FALSE,
        91,
        "e.select { _1.cond? }\n .join\na = e.select { _1.cond? }\n .join\ne.select { _1.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { _1.cond? }.join\na = e.select { _1.cond? }.join\ne.select { _1.cond? }.join + []\n",
    ),
    (
        FALSE,
        102,
        "e.select { it.cond? }\n .join\na = e.select { it.cond? }\n .join\ne.select { it.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { it.cond? }.join\na = e.select { it.cond? }.join\ne.select { it.cond? }.join + []\n",
    ),
    (
        FALSE,
        115,
        "a =\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        FALSE,
        126,
        "a =\n  case x\n  when :a\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        FALSE,
        138,
        "a +\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (FALSE, 149, "a do\n  x\n  y\nend\n", &[], ""),
    (FALSE, 158, "a ||= begin\n  x\n  y\nend\n", &[], ""),
    (FALSE, 167, "x def self.y\n    z\n  end\n", &[], ""),
    (FALSE, 175, "my_method(1, 2, \"x\")\n", &[], ""),
    (
        FALSE,
        181,
        "my_method(1) \\\n  [:a]\n",
        &[(1, 1)],
        "my_method(1) [:a]\n",
    ),
    (FALSE, 193, "hash[:foo] \\\n  [:bar]\n", &[], ""),
    (
        FALSE,
        200,
        "{\n  key: value\n}[key]\n",
        &[(1, 1)],
        "{ key: value }[key]\n",
    ),
    (FALSE, 213, "foo && \\\n  bar\n", &[(1, 1)], "foo && bar\n"),
    (FALSE, 225, "foo \\\n  && bar\n", &[], ""),
    (FALSE, 232, "foo || \\\n  bar\n", &[(1, 1)], "foo || bar\n"),
    (FALSE, 244, "foo \\\n  || bar\n", &[], ""),
    (
        FALSE,
        293,
        "my_method(1,\n          2,\n          \"x\")\n",
        &[(1, 1)],
        "my_method(1, 2, \"x\")\n",
    ),
    (
        FALSE,
        306,
        "some_array.map do |something|\n  my_method(\n    something,\n  )\nend\n",
        &[(2, 3)],
        "some_array.map do |something|\n  my_method( something, )\nend\n",
    ),
    (
        FALSE,
        323,
        "my_method(1,\n          2,\n          \"x\") # X\n",
        &[],
        "",
    ),
    (
        FALSE,
        331,
        "my_method(\"a\" \\\n          \"b\")\n",
        &[(1, 1)],
        "my_method(\"ab\")\n",
    ),
    (
        FALSE,
        343,
        "puts \"(#{pl(i)}, \" \\\n     \"#{pl(f)})\"\n",
        &[(1, 1)],
        "puts \"(#{pl(i)}, #{pl(f)})\"\n",
    ),
    (
        FALSE,
        355,
        "my_method('a'\\\n          'b')\n",
        &[(1, 1)],
        "my_method('ab')\n",
    ),
    (
        FALSE,
        367,
        "my_method(\"a\" \\\n          'b')\nmy_method('a' \\\n          \"b\")\n",
        &[(1, 1), (3, 1)],
        HELD,
    ),
    (
        FALSE,
        383,
        "my_method(1 +\n          2 +\n          3)\n",
        &[(1, 1)],
        "my_method(1 + 2 + 3)\n",
    ),
    (
        FALSE,
        396,
        "a =\n  m(1 +\n    2 +\n    3)\nb = m(4 +\n      5 +\n      6)\nlong_variable_name =\n  m(7 +\n    8 +\n    9)\n",
        &[(1, 1), (5, 1), (8, 1)],
        "a = m(1 + 2 + 3)\nb = m(4 + 5 + 6)\nlong_variable_name = m(7 + 8 + 9)\n",
    ),
    (
        FALSE,
        422,
        "a = b ||= {\n  x: 1\n}\n",
        &[(1, 1)],
        "a = b ||= { x: 1 }\n",
    ),
    (
        FALSE,
        254,
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError,\n            \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
        &[(5, 7)],
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError, \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
    ),
    (
        FALSE,
        436,
        "foo(' .x')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(' .x').bar.baz\n",
    ),
    (
        FALSE,
        449,
        "foo(x,\n    y,\n    z)\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(x, y, z).bar.baz\n",
    ),
    (
        FALSE,
        464,
        "foo('....' \\\n    '....')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo('........').bar.baz\n",
    ),
    (
        FALSE,
        478,
        "foo(<<~EOS)\n  xyz\nEOS\n  .bar\n  .baz\n",
        &[],
        "",
    ),
    (FALSE, 488, "foo('\n  xyz\n')\n  .bar\n  .baz\n", &[], ""),
    (FALSE, 498, "%\n\n", &[], ""),
    (FALSE, 502, "x = %\n\n", &[], ""),
    (
        FALSE,
        509,
        "my_method(11111,\n          22222,\n          \"abcxyz\")\nmy_method(111111 +\n          222222 +\n          333333)\n",
        &[(1, 1), (4, 1)],
        "my_method(11111, 22222, \"abcxyz\")\nmy_method(111111 + 222222 + 333333)\n",
    ),
    (FALSE, 520, "foo(:\"\n\")\n", &[], ""),
    (
        FALSE,
        530,
        "VariableReference = Struct.new(:name) do\n  def assignment?\n    false\n  end\nend\n",
        &[],
        "",
    ),
    (
        FALSE,
        540,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) && a && File.basename(path).start_with?('.') && !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        FALSE,
        546,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) + a + File.basename(path).start_with?('.') + !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        FALSE,
        555,
        "correct = lambda do\n  expect_no_offenses(<<~EOT1)\n    <<-EOT2\n    foo\n    EOT2\n  EOT1\nend\n",
        &[],
        "",
    ),
    (
        FALSE,
        567,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { |n| n.loc.heredoc_body }\n    .flat_map { |b| (b.line...b.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        FALSE,
        576,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { _1.loc.heredoc_body }\n    .flat_map {  (_1.line..._1.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        FALSE,
        585,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345678' do\nend\n",
        &[(1, 1)],
        "context 'when the configuration includes an unsafe cop that is 123456789012345678' do\nend\n",
    ),
    (
        FALSE,
        593,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345' do\nend\n",
        &[(1, 1)],
        "context 'when the configuration includes an unsafe cop that is 123456789012345' do\nend\n",
    ),
    (
        FALSE,
        608,
        "RSpec.shared_context 'ruby 2.4', :ruby24 do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        FALSE,
        616,
        "do_something do\nrescue CustomError\nend\n",
        &[],
        "",
    ),
    (FALSE, 624, "do_something do\nensure\nend\n", &[], ""),
    (
        FALSE,
        747,
        "a = RSpec.shared_context('ruby 2.4', :ruby24) do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        FALSE,
        755,
        "a = Foo.do_something(arg) do\n  _1\nend\n",
        &[],
        "",
    ),
    (
        FALSE,
        763,
        "RSpec.shared_context do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (FALSE, 774, "f do\nend\n", &[], ""),
    (FALSE, 781, "f do\n  foo(_1)\nend\n", &[], ""),
    (FALSE, 793, "f do\nend\n", &[], ""),
    (FALSE, 800, "f do\n  foo(_1)\nend\n", &[], ""),
    (
        FALSE,
        808,
        "e.select do |i|\n  i.cond?\nend.join\na = e.select do |i|\n  i.cond?\nend.join\ne.select do |i|\n  i.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        FALSE,
        822,
        "e.select do\n  _1.cond?\nend.join\na = e.select do\n  _1.cond?\nend.join\ne.select do\n  _1.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        FALSE,
        836,
        "e.select do\n  it.cond?\nend.join\na = e.select do\n  it.cond?\nend.join\ne.select do\n  it.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        MAIN,
        856,
        "do_something(foo(a,\n                 b) => bar(c,\n                           d))\n",
        &[(1, 1)],
        "do_something(foo(a, b) => bar(c, d))\n",
    ),
    (
        MAIN,
        870,
        "[foo(a,\n     b), bar(c,\n             d)]\n",
        &[(1, 2), (2, 10)],
        "[foo(a, b), bar(c, d)]\n",
    ),
    (
        MAIN,
        884,
        "[fooo(a,\n      b), baar(c,\n               d), baaz(e,\n                        f)]\n",
        &[(1, 2), (2, 11), (3, 20)],
        "[fooo(a, b), baar(c, d), baaz(e, f)]\n",
    ),
    (
        MAIN,
        904,
        "my_method(1,\n          2,\n          \"x\")\n",
        &[(1, 1)],
        "my_method(1, 2, \"x\")\n",
    ),
];

/// `(spec line, max, SingleLineBlockChain enabled, source, annotated
/// findings, expect_correction)`; an empty correction means none is given.
type Spec = (
    u32,
    Option<usize>,
    bool,
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const SPEC: &[Spec] = &[
    (
        24,
        Some(90),
        false,
        "e.select { |i| i.cond? }\n  .join\na = e.select { |i| i.cond? }\n  .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e.select { |i| i.cond? }.join\na = e.select { |i| i.cond? }.join\ne.select { |i| i.cond? }.join + []\n",
    ),
    (
        44,
        Some(90),
        false,
        "e&.select { |i| i.cond? }\n  &.join\na = e&.select { |i| i.cond? }\n  &.join\ne&.select { |i| i.cond? }\n  &.join + []\n",
        &[(1, 1), (3, 1), (5, 1)],
        "e&.select { |i| i.cond? }&.join\na = e&.select { |i| i.cond? }&.join\ne&.select { |i| i.cond? }&.join + []\n",
    ),
    (
        69,
        Some(90),
        true,
        "e.select { |i| i.cond? }\n .join\na = e.select { |i| i.cond? }\n     .join\ne.select { |i| i.cond? }\n .join + []\n",
        &[],
        "",
    ),
    (
        80,
        Some(90),
        true,
        "e.select { |i| i.cond? }\n &.join\na = e.select { |i| i.cond? }\n     &.join\ne.select { |i| i.cond? }\n &.join + []\n",
        &[],
        "",
    ),
    (
        91,
        Some(90),
        true,
        "e.select { _1.cond? }\n .join\na = e.select { _1.cond? }\n .join\ne.select { _1.cond? }\n .join + []\n",
        &[],
        "",
    ),
    (
        102,
        Some(90),
        true,
        "e.select { it.cond? }\n .join\na = e.select { it.cond? }\n .join\ne.select { it.cond? }\n .join + []\n",
        &[],
        "",
    ),
    (
        115,
        Some(31),
        true,
        "a =\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        126,
        Some(31),
        true,
        "a =\n  case x\n  when :a\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (
        138,
        Some(31),
        true,
        "a +\n  if x\n    1\n  else\n    2\n  end\n",
        &[],
        "",
    ),
    (149, Some(31), true, "a do\n  x\n  y\nend\n", &[], ""),
    (158, Some(31), true, "a ||= begin\n  x\n  y\nend\n", &[], ""),
    (167, Some(31), true, "x def self.y\n    z\n  end\n", &[], ""),
    (175, Some(31), true, "my_method(1, 2, \"x\")\n", &[], ""),
    (
        181,
        Some(31),
        true,
        "my_method(1) \\\n  [:a]\n",
        &[(1, 1)],
        "my_method(1) [:a]\n",
    ),
    (193, Some(31), true, "hash[:foo] \\\n  [:bar]\n", &[], ""),
    (
        200,
        Some(31),
        true,
        "{\n  key: value\n}[key]\n",
        &[(1, 1)],
        "{ key: value }[key]\n",
    ),
    (
        213,
        Some(31),
        true,
        "foo && \\\n  bar\n",
        &[(1, 1)],
        "foo && bar\n",
    ),
    (225, Some(31), true, "foo \\\n  && bar\n", &[], ""),
    (
        232,
        Some(31),
        true,
        "foo || \\\n  bar\n",
        &[(1, 1)],
        "foo || bar\n",
    ),
    (244, Some(31), true, "foo \\\n  || bar\n", &[], ""),
    (
        293,
        Some(31),
        true,
        "my_method(1,\n          2,\n          \"x\")\n",
        &[(1, 1)],
        "my_method(1, 2, \"x\")\n",
    ),
    (
        306,
        Some(31),
        true,
        "some_array.map do |something|\n  my_method(\n    something,\n  )\nend\n",
        &[(2, 3)],
        "some_array.map do |something|\n  my_method( something, )\nend\n",
    ),
    (
        323,
        Some(31),
        true,
        "my_method(1,\n          2,\n          \"x\") # X\n",
        &[],
        "",
    ),
    (
        331,
        Some(31),
        true,
        "my_method(\"a\" \\\n          \"b\")\n",
        &[(1, 1)],
        "my_method(\"ab\")\n",
    ),
    (
        343,
        Some(31),
        true,
        "puts \"(#{pl(i)}, \" \\\n     \"#{pl(f)})\"\n",
        &[(1, 1)],
        "puts \"(#{pl(i)}, #{pl(f)})\"\n",
    ),
    (
        355,
        Some(31),
        true,
        "my_method('a'\\\n          'b')\n",
        &[(1, 1)],
        "my_method('ab')\n",
    ),
    (
        367,
        Some(31),
        true,
        "my_method(\"a\" \\\n          'b')\nmy_method('a' \\\n          \"b\")\n",
        &[(1, 1), (3, 1)],
        "my_method(\"a\" + 'b')\nmy_method('a' + \"b\")\n",
    ),
    (
        383,
        Some(31),
        true,
        "my_method(1 +\n          2 +\n          3)\n",
        &[(1, 1)],
        "my_method(1 + 2 + 3)\n",
    ),
    (
        396,
        Some(31),
        true,
        "a =\n  m(1 +\n    2 +\n    3)\nb = m(4 +\n      5 +\n      6)\nlong_variable_name =\n  m(7 +\n    8 +\n    9)\n",
        &[(1, 1), (5, 1), (9, 3)],
        "a = m(1 + 2 + 3)\nb = m(4 + 5 + 6)\nlong_variable_name =\n  m(7 + 8 + 9)\n",
    ),
    (
        422,
        Some(31),
        true,
        "a = b ||= {\n  x: 1\n}\n",
        &[(1, 1)],
        "a = b ||= { x: 1 }\n",
    ),
    (
        254,
        Some(100),
        true,
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError,\n            \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
        &[(5, 7)],
        "def resolve_inheritance_from_gems(hash)\n  gems = hash.delete('inherit_gem')\n  (gems || {}).each_pair do |gem_name, config_path|\n    if gem_name == 'rubocop'\n      raise ArgumentError, \"can't inherit configuration from the rubocop gem\"\n    end\n\n    hash['inherit_from'] = Array(hash['inherit_from'])\n    Array(config_path).reverse_each do |path|\n      # Put gem configuration first so local configuration overrides it.\n      hash['inherit_from'].unshift gem_config_path(gem_name, path)\n    end\n  end\nend\n",
    ),
    (
        436,
        Some(31),
        true,
        "foo(' .x')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(' .x').bar.baz\n",
    ),
    (
        449,
        Some(31),
        true,
        "foo(x,\n    y,\n    z)\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo(x, y, z).bar.baz\n",
    ),
    (
        464,
        Some(31),
        true,
        "foo('....' \\\n    '....')\n  .bar\n  .baz\n",
        &[(1, 1)],
        "foo('........').bar.baz\n",
    ),
    (
        478,
        Some(31),
        true,
        "foo(<<~EOS)\n  xyz\nEOS\n  .bar\n  .baz\n",
        &[],
        "",
    ),
    (
        488,
        Some(31),
        true,
        "foo('\n  xyz\n')\n  .bar\n  .baz\n",
        &[],
        "",
    ),
    (498, Some(31), true, "%\n\n", &[], ""),
    (502, Some(31), true, "x = %\n\n", &[], ""),
    (
        509,
        Some(31),
        true,
        "my_method(11111,\n          22222,\n          \"abcxyz\")\nmy_method(111111 +\n          222222 +\n          333333)\n",
        &[],
        "",
    ),
    (520, Some(31), true, "foo(:\"\n\")\n", &[], ""),
    (
        530,
        Some(82),
        true,
        "VariableReference = Struct.new(:name) do\n  def assignment?\n    false\n  end\nend\n",
        &[],
        "",
    ),
    (
        540,
        Some(82),
        true,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) && a && File.basename(path).start_with?('.') && !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        540,
        Some(82),
        true,
        "File.fnmatch?(\n  pattern, path,\n  File::FNM_PATHNAME | File::FNM_EXTGLOB\n) + a + File.basename(path).start_with?('.') + !hidden_dir?(path)\n",
        &[],
        "",
    ),
    (
        555,
        Some(82),
        true,
        "correct = lambda do\n  expect_no_offenses(<<~EOT1)\n    <<-EOT2\n    foo\n    EOT2\n  EOT1\nend\n",
        &[],
        "",
    ),
    (
        567,
        Some(82),
        true,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { |n| n.loc.heredoc_body }\n    .flat_map { |b| (b.line...b.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        576,
        Some(82),
        true,
        "node.each_node(:dstr)\n    .select(&:heredoc?)\n    .map { _1.loc.heredoc_body }\n    .flat_map {  (_1.line..._1.last_line).to_a }\n",
        &[],
        "",
    ),
    (
        585,
        Some(82),
        true,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345678' do\nend\n",
        &[],
        "",
    ),
    (
        593,
        Some(82),
        true,
        "context 'when the configuration includes ' \\\n        'an unsafe cop that is 123456789012345' do\nend\n",
        &[(1, 1)],
        "context 'when the configuration includes an unsafe cop that is 123456789012345' do\nend\n",
    ),
    (
        608,
        Some(82),
        true,
        "RSpec.shared_context 'ruby 2.4', :ruby24 do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        616,
        Some(82),
        true,
        "do_something do\nrescue CustomError\nend\n",
        &[],
        "",
    ),
    (
        624,
        Some(82),
        true,
        "do_something do\nensure\nend\n",
        &[],
        "",
    ),
    (
        747,
        Some(100),
        true,
        "a = RSpec.shared_context('ruby 2.4', :ruby24) do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (
        755,
        Some(100),
        true,
        "a = Foo.do_something(arg) do\n  _1\nend\n",
        &[],
        "",
    ),
    (
        763,
        Some(100),
        true,
        "RSpec.shared_context do\n  let(:ruby_version) { 2.4 }\nend\n",
        &[],
        "",
    ),
    (774, Some(100), true, "f do\nend\n", &[], ""),
    (781, Some(100), true, "f do\n  foo(_1)\nend\n", &[], ""),
    (793, Some(100), false, "f do\nend\n", &[], ""),
    (800, Some(100), false, "f do\n  foo(_1)\nend\n", &[], ""),
    (
        808,
        Some(100),
        false,
        "e.select do |i|\n  i.cond?\nend.join\na = e.select do |i|\n  i.cond?\nend.join\ne.select do |i|\n  i.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        822,
        Some(100),
        false,
        "e.select do\n  _1.cond?\nend.join\na = e.select do\n  _1.cond?\nend.join\ne.select do\n  _1.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        836,
        Some(100),
        false,
        "e.select do\n  it.cond?\nend.join\na = e.select do\n  it.cond?\nend.join\ne.select do\n  it.cond?\nend.join + []\n",
        &[],
        "",
    ),
    (
        856,
        Some(35),
        true,
        "do_something(foo(a,\n                 b) => bar(c,\n                           d))\n",
        &[(1, 14)],
        "do_something(foo(a, b) => bar(c,\n                           d))\n",
    ),
    (
        870,
        Some(35),
        true,
        "[foo(a,\n     b), bar(c,\n             d)]\n",
        &[(1, 2), (2, 10)],
        "[foo(a, b), bar(c, d)]\n",
    ),
    (
        884,
        Some(35),
        true,
        "[fooo(a,\n      b), baar(c,\n               d), baaz(e,\n                        f)]\n",
        &[(1, 2), (2, 11)],
        "[fooo(a, b), baar(c, d), baaz(e,\n                        f)]\n",
    ),
    (
        904,
        None,
        true,
        "my_method(1,\n          2,\n          \"x\")\n",
        &[(1, 1)],
        "my_method(1, 2, \"x\")\n",
    ),
];

fn cop(max: Option<usize>, chain: bool) -> RedundantLineBreak {
    RedundantLineBreak {
        max_line_length: max,
        single_line_block_chain: chain,
    }
}

/// `(line, col)` of each finding, checking the message, severity, the
/// highlight's last line and that no edit is offered without a correction
/// request.
fn findings_with(cop: &RedundantLineBreak, source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[cop], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert!(o.end_line > o.line);
            (o.line, o.col)
        })
        .collect();
    found.sort();
    found
}

fn findings(source: &str) -> Vec<(u32, u32)> {
    findings_with(&RedundantLineBreak::default(), source)
}

fn fixed_with(cop: &RedundantLineBreak, source: &str) -> String {
    let offenses = correction_offenses(cop, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

fn fixed(source: &str) -> String {
    fixed_with(&RedundantLineBreak::default(), source)
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&RedundantLineBreak::default(), source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 123);
    for &(context, line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "{context} spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&RedundantLineBreak::default(), source).is_empty());
        } else if corrected == HELD {
            held(source);
        } else {
            assert_correction(&RedundantLineBreak::default(), source, corrected);
        }
    }
}

#[test]
fn spec_configurations_match_the_annotations() {
    assert_eq!(SPEC.len(), 65);
    for &(line, max, chain, source, expected, corrected) in SPEC {
        let cop = cop(max, chain);
        let mut expected = expected.to_vec();
        expected.sort();
        assert_eq!(findings_with(&cop, source), expected, "spec line {line}");
        if !corrected.is_empty() && line != 367 {
            assert_eq!(fixed_with(&cop, source), corrected, "spec line {line}");
        }
    }
}

#[test]
fn opt_in_metadata() {
    let cop = RedundantLineBreak::default();
    assert!(!cop.default_enabled());
    assert!(!cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert_eq!(cop.max_line_length, Some(120));
    assert!(!cop.single_line_block_chain);
}

#[test]
fn configuration_seams_keep_each_other() {
    let registry = crate::cops::Registry::new();
    for (toml, max, chain) in [
        ("", Some(120), false),
        ("[cops.\"Layout/LineLength\"]\nmax = 80\n", Some(80), false),
        (
            "[cops.\"Layout/LineLength\"]\nenabled = false\n",
            None,
            false,
        ),
        (
            "[cops.\"Layout/SingleLineBlockChain\"]\nenabled = true\n[cops.\"Layout/LineLength\"]\nmax = 90\n",
            Some(90),
            true,
        ),
        // A department setting never enables the opt-in chain cop.
        ("[cops.Layout]\nenabled = true\n", Some(120), false),
    ] {
        let config =
            crate::config::ResolvedConfig::resolve(toml::from_str(toml).unwrap(), None, &registry)
                .unwrap();
        let configured = config.configured.get(COP).unwrap();
        // Observe both settings through behaviour: a chain on a single-line
        // block (deferred when the chain cop is on) and a line of 85.
        let chained = "e.select { |i| i.cond? }\n  .join\n";
        let found = lint_str_with(&[configured.as_ref()], chained);
        assert_eq!(found.is_empty(), chain, "{toml}");
        let long = format!("foo(1,\n  {})\n", "x".repeat(78));
        let found = lint_str_with(&[configured.as_ref()], &long);
        assert_eq!(
            found.len(),
            usize::from(max.is_none_or(|m| m >= 85)),
            "{toml}"
        );
    }
}

#[test]
fn to_single_line_ports_the_five_substitutions() {
    for (before, after) in [
        ("foo(1,\n  2)", "foo(1, 2)"),
        ("a \\\n  + b", "a + b"),
        ("foo\n  .bar\n  &.baz", "foo.bar&.baz"),
        ("foo\n  . bar", "foo . bar"),
        ("'a' \\\n  'b'", "'ab'"),
        ("\"a\" \\\n  \"b\"", "\"ab\""),
        ("\"a\" \\\n  'b'", "\"a\" + 'b'"),
        ("'a'\\\n'b'", "'ab'"),
        ("'a' \\\n  \"b\"", "'a' + \"b\""),
        ("a  \n\n  b", "a b"),
        ("a\r\n  b", "a b"),
        ("é(1,\n  2)", "é(1, 2)"),
    ] {
        assert_eq!(
            String::from_utf8(to_single_line(before.as_bytes())).unwrap(),
            after,
            "{before:?}"
        );
    }
}

#[test]
fn candidates_follow_upstreams_callbacks() {
    for (source, expected) in [
        // Every assignment kind.
        ("@a = foo(1,\n  2)\n", vec![(1, 1)]),
        ("$a ||= foo(1,\n  2)\n", vec![(1, 1)]),
        ("A::B = foo(1,\n  2)\n", vec![(1, 1)]),
        ("a, b = foo(1,\n  2)\n", vec![(1, 1)]),
        ("x.y += foo(1,\n  2)\n", vec![(1, 1)]),
        ("x[0] &&= foo(1,\n  2)\n", vec![(1, 1)]),
        // Sends climb through receivers, arguments, `&&`/`||` and
        // parenthesized or argument-less literal blocks; not through
        // csends, splats, pairs or block passes (where the outer send,
        // visited first, is the offense).
        ("foo(bar(1,\n  2))\n", vec![(1, 1)]),
        ("x&.foo(bar(1,\n  2))\n", vec![(1, 1)]),
        ("foo(*bar(1,\n  2))\n", vec![(1, 1)]),
        ("foo(k: bar(1,\n  2))\n", vec![(1, 1)]),
        ("x = 1 + y&.foo(bar(1,\n  2))\n", vec![(1, 1)]),
        ("[y&.foo(bar(1,\n  2))]\n", vec![(1, 2)]),
        ("[y&.foo(1).bar(2,\n  3)]\n", vec![(1, 2)]),
        ("[foo(*bar(1,\n  2) { })]\n", vec![]),
        ("[foo(1) { }.bar(2,\n  3)]\n", vec![(1, 2)]),
        // Climbing to an `&&` without a backslash after it: nothing.
        ("foo && bar(1,\n  2)\n", vec![]),
        ("foo && \\\n  bar(1, 2)\n", vec![(1, 1)]),
        ("x[bar(1,\n  2)]\n", vec![(1, 1)]),
        ("!bar(1,\n  2)\n", vec![(1, 1)]),
        // A `[]` call on a `[]` call is never joined.
        ("x[1] \\\n  [2]\n", vec![]),
        // `and`/`or` only with a backslash after the operator.
        ("foo and \\\n  bar\n", vec![(1, 1)]),
        ("foo or\n  bar\n", vec![]),
        // Blocks are not inspected; nor is a send holding a multi-line one.
        ("foo(1,\n  2) { |x| x }\n", vec![]),
        ("foo(1,\n  bar do\n  end)\n", vec![]),
        ("foo(\n  -> { 1 }\n)\n", vec![(1, 1)]),
        // Comments, conditionals, definitions, heredocs and literal line
        // breaks are not split.
        ("foo(1, # c\n  2)\n", vec![]),
        ("foo(x ? 1 : 2,\n  3)\n", vec![]),
        ("foo(\"a\\nb\",\n  3)\n", vec![]),
        ("foo(?\\n,\n  3)\n", vec![]),
        ("foo(/a\n/,\n  3)\n", vec![]),
        ("foo(:\"a\nb\")\n", vec![]),
        ("foo((1;\n  2))\n", vec![]),
        // The multi-line interpolation is a `begin`: `foo` is not split, but
        // the `+` inside (whose parent is that `begin`) is.
        ("foo(\"#{1 +\n  2}\")\n", vec![(1, 8)]),
        ("foo(<<~A,\n  x\nA\n  2)\n", vec![]),
        ("foo(1 rescue 2,\n  3)\n", vec![]),
        // `%w` separators, regexp escapes and escaped line breaks join.
        ("foo(%w[a\n  b])\n", vec![(1, 1)]),
        ("foo(/a\\nb/,\n  3)\n", vec![(1, 1)]),
        // The join must reparse to the same tree: a line continuation in a
        // double-quoted string changes its value.
        ("foo(\"abc \\\n  def\")\n", vec![]),
        // In a `def`, the fragment is reparsed.
        ("def f\n  foo(1,\n    2)\nend\n", vec![(2, 3)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
    }
}

#[test]
fn line_values_are_neutralized_for_detection_and_held_for_correction() {
    // Upstream compares `__LINE__` as an identifier: reported, but joining
    // would change the value, so the edit is withheld.
    for source in [
        "foo(__LINE__,\n  __LINE__)\n",
        "def f\n  foo(1,\n    __LINE__)\nend\n",
    ] {
        assert_eq!(findings(source).len(), 1, "{source}");
        held(source);
    }
    // A `__LINE__` after the edit is held by the shared guard.
    held("foo(1,\n  2)\np __LINE__\n");
    assert_eq!(
        fixed("p __LINE__\nfoo(1,\n  2)\n"),
        "p __LINE__\nfoo(1, 2)\n"
    );
}

#[test]
fn unsafe_or_inexact_joins_are_withheld() {
    for source in [
        // A split string would become a `+` call (a new, unfrozen string).
        "x = \"a\" \\\n  'b'\n",
        "x = 'a' \\\n  \"b\"\n",
        // the listed source observers.
        "foo(1,\n  2)\np __FILE__\n",
        "foo(1,\n  2)\np __dir__\n",
        "foo(1,\n  2)\np $0\n",
        "foo(1,\n  2)\np $PROGRAM_NAME\n",
        "foo(1,\n  2)\np caller\n",
        "foo(1,\n  2)\np caller_locations\n",
        "foo(1,\n  2)\np m.source_location\n",
        "foo(1,\n  2)\np SCRIPT_LINES__\n",
        "foo(1,\n  2)\np DATA.read\n__END__\nx\n",
    ] {
        held(source);
    }
}

#[test]
fn corrections_are_upstreams_join() {
    for (before, after) in [
        ("x = foo(1,\n  2)\n", "x = foo(1, 2)\n"),
        ("foo(\n  1,\n)\n", "foo( 1, )\n"),
        ("x = 'a' \\\n  'b'\n", "x = 'ab'\n"),
        ("foo(%w[a\n  b])\n", "foo(%w[a b])\n"),
        ("foo\n  .bar(1)\n  &.baz\n", "foo.bar(1)&.baz\n"),
        ("foo(1,\r\n  2)\r\n", "foo(1, 2)\r\n"),
        // Upstream's chain substitution would leave `x = a\r.b`.
        ("x = a\r\n  .b\r\n  .c\r\n", "x = a.b.c\r\n"),
        ("\u{feff}foo(1,\n  2)\n", "\u{feff}foo(1, 2)\n"),
        (
            "def f\n  foo(1,\n    2)\nend\n",
            "def f\n  foo(1, 2)\nend\n",
        ),
    ] {
        assert_correction(&RedundantLineBreak::default(), before, after);
    }
}

#[test]
fn shared_lines_fit_together_and_line_length_counts_characters() {
    // Upstream's `joined_line_range`: the second expression is checked
    // with the first one's lines.
    let cop35 = cop(Some(35), true);
    let source =
        "do_something(foo(a,\n                 b) => bar(c,\n                           d))\n";
    assert_eq!(findings_with(&cop35, source), [(1, 14)]);
    // Characters, not bytes.
    let source = "foo(1,\n  \"éééé\")\n";
    assert_eq!(findings_with(&cop(Some(14), false), source), [(1, 1)]);
    assert_eq!(findings_with(&cop(Some(13), false), source), []);
}

#[test]
fn a_file_ending_in_percent_blank_skips_local_assignments_only() {
    assert_eq!(findings("x = %\n\n"), []);
    assert_eq!(findings("x = foo(1,\n  2)\ny = %\n\n"), [(1, 5)]);
    assert_eq!(findings("@x = foo(1,\n  2)\ny = %\n\n"), [(1, 1)]);
}
