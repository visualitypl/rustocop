//! Examples ported from RuboCop's `spec/rubocop/cop/layout/multiline_method_call_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example of every style context (`aligned`, `indented`,
//! `indented_relative_to_receiver`, per-cop `IndentationWidth`, the shared
//! `common` groups and interpolated keyword/assignment groups) is replayed at
//! the fixed `aligned` style; findings, messages and single-pass corrected
//! bytes match the fixed replay recorded in
//! the pinned RuboCop reference run. Sources shared by several
//! examples are listed once with all their spec lines.
use super::*;
use crate::{cops::testing::assert_correction, pipeline::lint_str};

const COP: &str = "Layout/MultilineMethodCallIndentation";

/// Findings sorted by position (RuboCop reports outer sends first).
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    let mut found: Vec<_> = lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.message))
        .collect();
    found.sort();
    found
}

/// Every distinct pinned source without an offense at the fixed defaults,
/// with the spec lines that use it.
const ACCEPTED: &[(&[u32], &str)] = &[
    (&[956], "1 + a\n    .b\n    .c + d.\n         e\n"),
    (
        &[965],
        "def investigate(processed_source)\n  @modifier = processed_source\n              .tokens\n              .select { |t| t.type == :k }\n              .map(&:pos)\nend\n",
    ),
    (
        &[976],
        "KeyMap = Hash.new do |map, key|\n  value = if key.respond_to?(:to_str)\n    key\n  else\n    key.to_s.split('_').\n      each { |w| w.capitalize! }.\n      join('-')\n  end\n  keymap_mutex.synchronize { map[key] = value }\nend\n",
    ),
    (
        &[991],
        "expect { custom_formatter_class('NonExistentClass') }\n  .to raise_error(NameError)\n",
    ),
    (&[1015], "a.\n  b + c\n      .d\n"),
    (&[1023, 2249], "if a.\n   b\n  something\nend\n"),
    (
        &[1032],
        "@dependencies ||= begin\n  DEFAULT_DEPRUBYENCIES\n    .reject { |e| e }\n    .map { |e| e }\nend\n",
    ),
    (
        &[1059],
        "flash[:error] = 'Here is a string ' \\\n                'That spans' <<\n  'multiple lines'\n",
    ),
    (
        &[1067],
        "flash[:error] = 'Here is a string ' +\n                'That spans' <<\n  'multiple lines'\n",
    ),
    (&[1141], "def a\n  return b.\n         c\nend\n"),
    (&[1150], "a = b do\n  c.d = e.\n        f\nend\n"),
    (
        &[1159],
        "formatted_int = int_part\n                .to_s\n                .reverse\n                .gsub(/...(?=.)/, '&_')\n",
    ),
    (&[1183], "do_something.foo do\nend.bar\n   .baz\n"),
    (
        &[1191],
        "do_something.foo do\n  bar(_1)\nend.baz\n   .qux\n",
    ),
    (
        &[1200],
        "do_something.foo do\n  bar(it)\nend.baz\n   .qux\n",
    ),
    (&[1209], "do_something.foo do\nend&.bar\n   &.baz\n"),
    (
        &[1217],
        "users\n  .dup.sort_by { _1.name.lower }\n  .page(params[:page])\n  .per(PER_PAGE)\n",
    ),
    (
        &[1226],
        "users\n  &.dup&.sort_by { _1.name.lower }\n  &.page(params[:page])\n",
    ),
    (
        &[1234],
        "users\n  .dup.sort_by { _1.name }\n  .select { |u| u.active? }\n  .map(&:id)\n",
    ),
    (
        &[1243],
        "users\n  .dup.compact.sort_by { _1.name }\n  .first(10)\n",
    ),
    (
        &[1251],
        "obj\n  .foo { _1.a }.bar { _1.b }\n  .each do |x|\n  do_something(x)\nend\n",
    ),
    (&[1261], "obj\n  .foo { _1.a }.bar { _1.b }\n  .baz(x)\n"),
    (
        &[1269],
        "obj\n  &.foo { _1.a }&.bar { _1.b }\n  &.each do |x|\n  do_something(x)\nend\n",
    ),
    (&[1298], "(0..foo).bar { baz }\n        .qux { quux }\n"),
    (&[1305], "(0..foo).bar { baz }\n        .qux\n"),
    (&[1312], "(0..foo).bar\n        .qux { quux }\n"),
    (&[1319], "(0..foo).bar { baz }\n        &.qux { quux }\n"),
    (&[1378], "A = b\n    .c\n"),
    (&[1385], "a +=\n  b\n  .c\n"),
    (&[1578, 2052], "{ a: 1, b: 2 }.keys\n              .first\n"),
    (&[18], "a\n  .b[c] = 0\n"),
    (
        &[25],
        "a = b.map do |c|\n  c\n    .b\n    .d do\n      x\n        .y\n    end\nend\n",
    ),
    (
        &[38],
        "node\n  .children.map { |n| string_source(n) }.compact\n  .any? { |s| preferred.any? { |d| s.include?(d) } }\n",
    ),
    (&[46], "a.\n  b\n"),
    (&[53], "a.\n  b.\n  c\n"),
    (&[61], "for x in a\n  something.\n    something_else\nend\n"),
    (&[70], "(a.\n b)\n"),
    (&[77], "(\n  a * b do\n  end\n)\n  .c\n"),
    (
        &[87],
        "subject.each do |item|\n  result = resolve(locale) and return result\nend.a\n",
    ),
    (
        &[95],
        "payment = Models::IncomingPayments[\n        id:      input['incoming-payment-id'],\n           user_id: @user[:id]]\n",
    ),
    (&[103], "a(b)\n  .c(\n    d do\n    end.f\n  )\n"),
    (&[113], "MyClass.(my_args)\n  .my_method\n"),
    (
        &[120],
        "Foo\n  .do_something(\n    key: value do\n    end\n  )\n",
    ),
    (&[130], "query = x.|(\n  foo,\n  bar\n)\n"),
    (&[149], "(a +\n b)\n  .foo\n  .bar\n"),
    (
        &[158],
        "foo(\n  key => Model.joins(\n    Other\n      .arel_table\n      .join_sources\n  )\n)\n",
    ),
    (
        &[170],
        "foo(\n  key => (Other\n            .arel_table\n            .join_sources)\n)\n",
    ),
    (
        &[180],
        "{\n  key: -> do\n    client\n      .beta\n      .messages\n  end\n}\n",
    ),
    (
        &[192],
        "{\n  key: proc { |request|\n    request\n      .params\n      .fetch(:page)\n  }\n}\n",
    ),
    (
        &[204],
        "{\n  key: begin\n    settings\n      .fetch(:connection)\n      .fetch(:retries)\n  end\n}\n",
    ),
    (
        &[216],
        "{\n  key: if condition\n    client\n      .beta\n  else\n    other\n      .beta\n  end\n}\n",
    ),
    (
        &[230],
        "{\n  key: while condition\n    client\n      .beta\n  end\n}\n",
    ),
    (
        &[243],
        "expect { Foo.new }.\n  to change { Bar.count }.\n  from(1).to(2)\n",
    ),
    (&[462], "a&.(b)\n  .c(\n    d do\n    end.f\n  )\n"),
    (&[811], "User.all.first\n    .age.to_s\n"),
    (
        &[818],
        "authorize scope.includes(:user)\n               .where(name: 'Bob')\n               .order(:name)\n",
    ),
    (
        &[826],
        "do_something obj.foo(key: value)\n                &.bar(arg)\n",
    ),
    (
        &[855],
        "user = authorize scope.includes(:user)\n                      .where(name: 'Bob')\n                      .order(:name)\n",
    ),
    (&[863], "age = User.all.first\n          .age.to_s\n"),
    (&[870], "foo = '123'.a\n           .b[1]\n           .c\n"),
    (
        &[878],
        "foo = '123'[1].a\n              .b\n              .c\n",
    ),
    (&[886], "a[1][2][3]"),
    (
        &[890],
        "@comment_lines ||=\n  src.comments\n     .select { |c| begins_its_line?(c) }\n     .map { |c| c.loc.line }\n",
    ),
    (&[899], "hash[key] { 10 / 0 }\n  .fmap { |x| x * 3 }\n"),
    (
        &[906],
        "a_class.new(severity, location, 'message', 'CopName')\n       .severity\n       .level\n",
    ),
    (
        &[947],
        "expect { post :action, params: params, format: :json }.to change { Foo.bar }.by(0)\n                                                      .and change { Baz.quux }.by(0)\n                                                      .and raise_error(StandardError)\n",
    ),
    (&[562], "{\n  key: Foo.bar\n          .baz\n}\n"),
    (
        &[571],
        "{\n  key: Foo.bar\n          .baz\n          .qux\n}\n",
    ),
    (
        &[581],
        "{\n  outer: {\n    inner: Foo.bar\n              .baz\n  }\n}\n",
    ),
    (&[592], "method_call(\n  key: Foo.bar\n          .baz\n)\n"),
    (
        &[601],
        "method(key: value.foo.bar\n                 .baz)\n",
    ),
    (&[608], "{\n  key: value.foo.bar\n            .baz\n}\n"),
    (
        &[617],
        "method(key: value&.foo&.bar\n                 &.baz)\n",
    ),
    (
        &[669],
        "@foo = Foo\n       .where(id: Bar.select(:id)\n         .joins(:bar)\n         .where.not(bar: { id: 123 }))\n",
    ),
    (
        &[678],
        "@foo = Foo\n       &.where(id: Bar.select(:id)\n         &.joins(:bar))\n",
    ),
    (
        &[686],
        "@foo = Foo.\n       where(id: Bar.select(:id)\n         .joins(:bar))\n",
    ),
    (
        &[694, 2090, 2588],
        "Foo\n  .where(id: Bar.select(:id)\n    .joins(:bar),\n         name: Car.find(:name)\n    .strip)\n",
    ),
    (
        &[704, 2117, 2615],
        "Foo\n  .where({id: Bar.select(:id)\n    .joins(:bar)})\n",
    ),
    (
        &[712, 2100, 2598],
        "Foo\n  .where(id: Bar.select(:id)\n    .joins(:bar))\n  .order(:name)\n",
    ),
    (
        &[734],
        "timeseries.push(\n  {\n    value: timeseries_snapshots.pluck(:membership_gross_value)\n                               .compact_sum\n                               .to_f,\n  },\n)\n",
    ),
    (
        &[746],
        "{\n  alpha_beta_count: @handler.retrieve_all_by(alpha: alpha)\n                            .authorized.count,\n  beta_count: @handler.retrieve_all.authorized.count\n}\n",
    ),
    (
        &[756],
        "filter :season, apply: lambda { |records, values, _options|\n  return records if values.blank?\n\n  records.joins(:match)\n         .where(matches: { season_id: values })\n}\n",
    ),
    (&[769], "{\n  key: Foo.bar { |x| x }\n          .baz\n}\n"),
    (
        &[778],
        "{\n  key: Foo.bar do |x|\n    x\n  end.baz\n     .qux\n}\n",
    ),
    (
        &[835],
        "File.read('data.yml')\n    .then { YAML.safe_load _1 }\n    .transform_values(&:downcase)\n",
    ),
    (
        &[846],
        "File.read('data.yml')\n    .then { YAML.safe_load it }\n    .transform_values(&:downcase)\n",
    ),
    (&[1658], "hash[:key]\n  .do_something\n"),
    (&[1714], "[\n  *foo\n    .bar(\n      arg)\n]\n"),
    (&[1725], "[\n  *foo\n    .bar { |arg| baz(arg) }\n]\n"),
    (&[1735], "[\n  *foo\n    .bar { baz(_1) }\n]\n"),
    (&[1745], "[\n  *foo\n    .bar { baz(it) }\n]\n"),
    (&[1755], "[\n  **foo\n    .bar(\n      arg)\n]\n"),
    (&[1766], "[\n  **foo\n    .bar { |arg| baz(arg) }\n]\n"),
    (&[1776], "[\n  **foo\n    .bar { baz(_1) }\n]\n"),
    (&[1786], "[\n  **foo\n    .bar { baz(it) }\n]\n"),
    (
        &[2010],
        "def run\n  (date_columns + candidate_columns).uniq\n                                    .select { |column_name| castable?(column_name) }\nend\n",
    ),
    (&[2232], "foo[\n  bar\n  .baz\n]\n"),
    (
        &[2266],
        "Parser::Source::Range.new(expr.source_buffer,\n                          begin_pos,\n                          begin_pos + line.length)\n",
    ),
    (&[2274], "a(b.\n    c\n.d)\n"),
    (
        &[2282],
        "arg_array.size == a.size && (\n  arg_array == a ||\n  arg_array.map(&:children) == a.map(&:children)\n)\n",
    ),
    (
        &[2417],
        "formatted_int = int_part\n                .abs\n                .reverse\n",
    ),
    (
        &[139],
        "def slugs(type, path_prefix)\n  expanded_links_item(type)\n    .reject { |item| item[\"base_path\"].nil? }\n    .map { |item| item[\"base_path\"].gsub(%r{^#{path_prefix}}, \"\") }\nend\n",
    ),
    (&[2323], "if a\n  something.\n    something_else\nend\n"),
    (&[2323], "unless a\n  something.\n    something_else\nend\n"),
    (&[2323], "while a\n  something.\n    something_else\nend\n"),
    (&[2323], "until a\n  something.\n    something_else\nend\n"),
    (
        &[2334],
        "return unless receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\n",
    ),
    (
        &[2334],
        "return if receiver.nil? &&\n    !args.empty? &&\n    FORBIDDEN_METHODS.include?(method_name)\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for (lines, source) in ACCEPTED {
        assert!(
            findings(source).is_empty(),
            "spec lines {lines:?}: {source}"
        );
    }
}

const USE_2: &str = "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.";

#[test]
fn metadata_is_fixed() {
    let cop = MultilineMethodCallIndentation;
    assert_eq!(cop.name(), COP);
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert!(cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn prism_dispatch_shapes_map_to_parser_sends() {
    for (source, expected, after) in [
        // The `send` inside an attribute op-assignment and a safe call.
        ("a\n.b += 1\n", (2, 1, USE_2), "a\n  .b += 1\n"),
        ("a\n.b ||= 1\n", (2, 1, USE_2), "a\n  .b ||= 1\n"),
        ("a\n&.b\n", (2, 1, USE_2), "a\n  &.b\n"),
        // An implicit `call`: `.(` is the right-hand side.
        ("a\n.(1)\n", (2, 1, USE_2), "a\n  .(1)\n"),
        // Bracket calls are `send :[]`: an index receiver is part of the chain.
        (
            "a[1].b\n  .c\n",
            (2, 3, "Align `.c` with `.b` on line 1."),
            "a[1].b\n    .c\n",
        ),
        // A trailing dot: the selector alone.
        ("a.\nb\n", (2, 1, USE_2), "a.\n  b\n"),
    ] {
        let found = findings(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].0, found[0].1, found[0].2.as_str()),
            expected,
            "{source}"
        );
        assert_correction(&MultilineMethodCallIndentation, source, after);
    }
    for source in [
        "a.b.c\n",
        "Foo\n::bar\n",
        "a\n  .b += 1\n",
        "foo[\n  1\n]\n",
        "x = [a\n  .b]\n",
        "a.b(c\n  .d)\n",
    ] {
        assert!(findings(source).is_empty(), "{source}");
    }
}

#[test]
fn block_chains_shift_the_selector_line_and_the_block() {
    let source = "a\n    .each do |x|\n      x\n    end\n";
    assert_eq!(
        findings(source),
        [(
            2,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.".to_owned()
        )]
    );
    assert_correction(
        &MultilineMethodCallIndentation,
        source,
        "a\n  .each do |x|\n    x\n  end\n",
    );
    // Numbered and `it` blocks are parser blocks too.
    for parameter in ["_1", "it"] {
        let source = format!("a\n    .each {{\n      {parameter}\n    }}\n");
        let after = format!("a\n  .each {{\n    {parameter}\n  }}\n");
        assert_correction(&MultilineMethodCallIndentation, &source, &after);
    }
}

#[test]
fn crlf_bom_and_wide_characters() {
    assert_correction(
        &MultilineMethodCallIndentation,
        "a\r\n.b\r\n",
        "a\r\n  .b\r\n",
    );
    let source = "\u{feff}a.b\n  .c\n";
    assert_eq!(
        findings(source),
        [(2, 3, "Align `.c` with `.b` on line 1.".to_owned())]
    );
    assert_correction(
        &MultilineMethodCallIndentation,
        source,
        "\u{feff}a.b\n .c\n",
    );
    // Parser columns are characters, not display columns.
    assert!(findings("'Ｒ'.b\n   .c\n").is_empty());
    assert_eq!(
        findings("'Ｒ'.b\n    .c\n"),
        [(2, 5, "Align `.c` with `.b` on line 1.".to_owned())]
    );
}
