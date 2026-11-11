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
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/MultilineMethodCallIndentation";

type Reported = (
    &'static [u32],
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

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

/// Every distinct pinned source with offenses: spec lines, source,
/// (line, column, message) findings and the single-pass correction.
const REPORTED: &[Reported] = &[
    (
        &[520],
        "{\n  key: -> do\n    client\n    .beta\n  end\n}\n",
        &[(
            4,
            5,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "{\n  key: -> do\n    client\n      .beta\n  end\n}\n",
    ),
    (
        &[541, 1633, 2134],
        "def foo\n  !0\n  .nil?\nend\n",
        &[(
            3,
            3,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "def foo\n  !0\n    .nil?\nend\n",
    ),
    (
        &[998, 2208],
        "a\n  .b\n .c\n",
        &[(
            3,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a\n  .b\n  .c\n",
    ),
    (
        &[1042, 2223],
        "if a.\n    b\n  something\nend\n",
        &[(2, 5, "Align `b` with `a.` on line 1.")],
        "if a.\n   b\n  something\nend\n",
    ),
    (
        &[1075],
        "flash[:error] = here_is_a_string.\n                that_spans.\n   multiple_lines\n",
        &[(
            3,
            4,
            "Align `multiple_lines` with `here_is_a_string.` on line 1.",
        )],
        "flash[:error] = here_is_a_string.\n                that_spans.\n                multiple_lines\n",
    ),
    (
        &[1090],
        "unless a\n.b\n  something\nend\n",
        &[(2, 1, "Align `.b` with `a` on line 1.")],
        "unless a\n       .b\n  something\nend\n",
    ),
    (
        &[1107],
        "while a.\n    b\n  something\nend\n",
        &[(2, 5, "Align `b` with `a.` on line 1.")],
        "while a.\n      b\n  something\nend\n",
    ),
    (
        &[1124],
        "until a.\n    b\n  something\nend\n",
        &[(2, 5, "Align `b` with `a.` on line 1.")],
        "until a.\n      b\n  something\nend\n",
    ),
    (
        &[1168],
        "do_something.foo do\nend.bar\n            .baz\n",
        &[(3, 13, "Align `.baz` with `.bar` on line 2.")],
        "do_something.foo do\nend.bar\n   .baz\n",
    ),
    (
        &[1279],
        "obj\n  .foo { _1.a }.bar { _1.b }\n    .each do |x|\n    do_something(x)\n  end\n",
        &[(3, 5, "Align `.each` with `.foo` on line 2.")],
        "obj\n  .foo { _1.a }.bar { _1.b }\n  .each do |x|\n  do_something(x)\nend\n",
    ),
    (
        &[1326, 2151],
        "(0..foo).bar { baz }\n  .qux { quux }\n",
        &[(2, 3, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar { baz }\n        .qux { quux }\n",
    ),
    (
        &[1339, 2158],
        "(0..foo).bar { baz }\n  .qux\n",
        &[(2, 3, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar { baz }\n        .qux\n",
    ),
    (
        &[1352, 2165],
        "(0..foo).bar\n  .qux { quux }\n",
        &[(2, 3, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar\n        .qux { quux }\n",
    ),
    (
        &[1365],
        "a = b.c.\n d\n",
        &[(2, 2, "Align `d` with `b.c.` on line 1.")],
        "a = b.c.\n    d\n",
    ),
    (
        &[1393],
        "bar = Foo\n  .a\n      .b(c)\n",
        &[(2, 3, "Align `.a` with `Foo` on line 1.")],
        "bar = Foo\n      .a\n      .b(c)\n",
    ),
    (
        &[1408, 1880, 2433],
        "def foo\n  bar(\n    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName\n .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name\n  )\nend\n",
        &[(
            4,
            2,
            "Align `.veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name` with `VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName` on line 3.",
        )],
        "def foo\n  bar(\n    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName\n         .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name\n  )\nend\n",
    ),
    (
        &[1429, 1901, 2454],
        "def foo\n  bar(\n    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName\n                .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name\n  )\nend\n",
        &[(
            4,
            17,
            "Align `.veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name` with `VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName` on line 3.",
        )],
        "def foo\n  bar(\n    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName\n         .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name\n  )\nend\n",
    ),
    (
        &[1450],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n              each_pair do |_, suggestions|\n                suggestions.transform_values! { |v| v == 'true' }\n              end\n).to_h\n",
        &[(
            3,
            15,
            "Align `each_pair` with `refusal_advice_params.fetch(:actions).` on line 2.",
        )],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n           each_pair do |_, suggestions|\n             suggestions.transform_values! { |v| v == 'true' }\n           end\n).to_h\n",
    ),
    (
        &[1471, 1943, 2496],
        "def payload\n  {\n    type: 'action',\n    params: {\n      page: get_page_name,\n      email: @action.member.email,\n      mailing_id: @mailing_id\n    }.reverse_merge(@action.form_data)\n      .merge(UserLanguageISO.for(page.language))\n      .tap do |params|\n        params[:country] = country(member.country) if member.country.present?\n        params[:action_bucket] = data[:bucket] if data.key? :bucket\n      end\n  }.deep_symbolize_keys\nend\n",
        &[
            (10, 7, "Align `.tap` with `.reverse_merge` on line 8."),
            (9, 7, "Align `.merge` with `.reverse_merge` on line 8."),
        ],
        "def payload\n  {\n    type: 'action',\n    params: {\n      page: get_page_name,\n      email: @action.member.email,\n      mailing_id: @mailing_id\n    }.reverse_merge(@action.form_data)\n     .merge(UserLanguageISO.for(page.language))\n     .tap do |params|\n       params[:country] = country(member.country) if member.country.present?\n       params[:action_bucket] = data[:bucket] if data.key? :bucket\n     end\n  }.deep_symbolize_keys\nend\n",
    ),
    (
        &[1511],
        "add_to_git_repo(\n  initial_repo,\n  \"about.json\" =>\n    JSON\n      .parse(about_json(about_url: \"https://updated.site.com\"))\n      .tap { |h| h[:component] = true }\n      .to_json,\n)\n",
        &[
            (7, 7, "Align `.to_json` with `JSON` on line 4."),
            (6, 7, "Align `.tap` with `JSON` on line 4."),
            (5, 7, "Align `.parse` with `JSON` on line 4."),
        ],
        "add_to_git_repo(\n  initial_repo,\n  \"about.json\" =>\n    JSON\n    .parse(about_json(about_url: \"https://updated.site.com\"))\n    .tap { |h| h[:component] = true }\n    .to_json,\n)\n",
    ),
    (
        &[1538, 2037],
        "(a +\n b).foo\n   .bar\n",
        &[(
            3,
            4,
            "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
        )],
        "(a +\n b).foo\n  .bar\n",
    ),
    (
        &[1553, 1613],
        "def targets_for(path)\n  fullpath = fullpath_for(path)\n  [\n    Dir.glob(fullpath),\n  ].flatten\n    .uniq\n    .delete_if { |entry| dot_directory?(entry) }\nend\n",
        &[(6, 5, "Align `.uniq` with `.flatten` on line 5.")],
        "def targets_for(path)\n  fullpath = fullpath_for(path)\n  [\n    Dir.glob(fullpath),\n  ].flatten\n   .uniq\n    .delete_if { |entry| dot_directory?(entry) }\nend\n",
    ),
    (
        &[1585],
        "{ a: 1, b: 2 }.keys\n  .first\n",
        &[(2, 3, "Align `.first` with `.keys` on line 1.")],
        "{ a: 1, b: 2 }.keys\n              .first\n",
    ),
    (
        &[251, 1672],
        "a.\nb\n",
        &[(
            2,
            1,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "a.\n  b\n",
    ),
    (
        &[264, 1829],
        "a.\n   b\nc.\n   d\n",
        &[
            (
                2,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
            (
                4,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
        ],
        "a.\n  b\nc.\n  d\n",
    ),
    (
        &[282, 1847],
        "a.\n  b.\n    c\n",
        &[(
            3,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.",
        )],
        "a.\n  b.\n  c\n",
    ),
    (
        &[298, 1863],
        "[\n a.\n b\n]\n",
        &[(
            3,
            2,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "[\n a.\n   b\n]\n",
    ),
    (
        &[315, 1685],
        "expect { Foo.new }.\n  to change { Bar.count }.\n      from(1).to(2)\n",
        &[(
            3,
            7,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "expect { Foo.new }.\n  to change { Bar.count }.\n  from(1).to(2)\n",
    ),
    (
        &[330, 1700],
        "a\n .(args)\n",
        &[(
            2,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a\n  .(args)\n",
    ),
    (
        &[343, 1816],
        "a\n .b\n",
        &[(
            2,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a\n  .b\n",
    ),
    // Held (finding kept, file unchanged).
    (
        &[486],
        "def run\n  (date_columns + candidate_columns).uniq\n                                    .select { |column_name|\n      castable?(column_name)\n    }\n    .each { |column_name|\n      cast(column_name)\n    }\nend\n",
        &[(
            3,
            37,
            "Use 2 (not 34) spaces for indenting an expression spanning multiple lines.",
        )],
        "def run\n  (date_columns + candidate_columns).uniq\n                                    .select { |column_name|\n      castable?(column_name)\n    }\n    .each { |column_name|\n      cast(column_name)\n    }\nend\n",
    ),
    (
        &[357],
        "a&.\nb\n",
        &[(
            2,
            1,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "a&.\n  b\n",
    ),
    (
        &[370],
        "a&.\n   b\nc&.\n   d\n",
        &[
            (
                2,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
            (
                4,
                4,
                "Use 2 (not 3) spaces for indenting an expression spanning multiple lines.",
            ),
        ],
        "a&.\n  b\nc&.\n  d\n",
    ),
    (
        &[388],
        "a&.\n  b&.\n    c\n",
        &[(
            3,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.",
        )],
        "a&.\n  b&.\n  c\n",
    ),
    (
        &[404],
        "[\n a&.\n b\n]\n",
        &[(
            3,
            2,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "[\n a&.\n   b\n]\n",
    ),
    (
        &[421],
        "expect { Foo.new }&.\n  to change { Bar.count }&.\n      from(1)&.to(2)\n",
        &[(
            3,
            7,
            "Use 2 (not 6) spaces for indenting an expression spanning multiple lines.",
        )],
        "expect { Foo.new }&.\n  to change { Bar.count }&.\n  from(1)&.to(2)\n",
    ),
    (
        &[436],
        "a\n &.(args)\n",
        &[(
            2,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a\n  &.(args)\n",
    ),
    (
        &[449],
        "a\n &.b\n",
        &[(
            2,
            2,
            "Use 2 (not 1) spaces for indenting an expression spanning multiple lines.",
        )],
        "a\n  &.b\n",
    ),
    (
        &[472],
        "MyClass.\nfoo&.bar = 'baz'\n",
        &[(
            2,
            1,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines.",
        )],
        "MyClass.\n  foo&.bar = 'baz'\n",
    ),
    (
        &[914],
        "User.a\n  .b\n .c\n",
        &[
            (3, 2, "Align `.c` with `.a` on line 1."),
            (2, 3, "Align `.b` with `.a` on line 1."),
        ],
        "User.a\n    .b\n    .c\n",
    ),
    (
        &[930],
        "a do\n  b.c\n    .d\nend\n",
        &[(3, 5, "Align `.d` with `.c` on line 2.")],
        "a do\n  b.c\n   .d\nend\n",
    ),
    // Held (finding kept, file unchanged).
    (
        &[624, 2065],
        "method(key: value.foo.bar\n              .baz)\n",
        &[(2, 15, "Align `.baz` with `.foo` on line 1.")],
        "method(key: value.foo.bar\n              .baz)\n",
    ),
    // Held (finding kept, file unchanged).
    (
        &[637],
        "method(key: value.foo.bar.\n                 baz)\n",
        &[(2, 18, "Align `baz` with `value.foo.bar.` on line 1.")],
        "method(key: value.foo.bar.\n                 baz)\n",
    ),
    (
        &[650],
        "{\n  key: Foo.bar\n       .baz\n}\n",
        &[(3, 8, "Align `.baz` with `.bar` on line 2.")],
        "{\n  key: Foo.bar\n          .baz\n}\n",
    ),
    (
        &[721],
        "Foo.where(id: Bar.select(:id)\n                        .joins(:bar))\n",
        &[(2, 25, "Align `.joins` with `.select` on line 1.")],
        "Foo.where(id: Bar.select(:id)\n                 .joins(:bar))\n",
    ),
    (
        &[789],
        "{\n  key: Foo.bar do |x|\n    x\n  end.baz\n        .qux\n}\n",
        &[(5, 9, "Align `.qux` with `.baz` on line 4.")],
        "{\n  key: Foo.bar do |x|\n    x\n  end.baz\n     .qux\n}\n",
    ),
    (
        &[1650],
        "1 + a\n      .b\n      .c\n",
        &[
            (3, 7, "Align `.c` with `a` on line 1."),
            (2, 7, "Align `.b` with `a` on line 1."),
        ],
        "1 + a\n    .b\n    .c\n",
    ),
    (
        &[1665],
        "expect { Foo.new }.to change { Bar.count }\n                        .from(1).to(2)\n",
        &[(
            2,
            25,
            "Use 2 (not 24) spaces for indenting an expression spanning multiple lines.",
        )],
        "expect { Foo.new }.to change { Bar.count }\n  .from(1).to(2)\n",
    ),
    (
        &[1795],
        "(0..foo).bar { baz }\n          .qux { quux }\n",
        &[(2, 11, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar { baz }\n        .qux { quux }\n",
    ),
    (
        &[1802],
        "(0..foo).bar { baz }\n          .qux\n",
        &[(2, 11, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar { baz }\n        .qux\n",
    ),
    (
        &[1809],
        "(0..foo).bar\n          .qux { quux }\n",
        &[(2, 11, "Align `.qux` with `.bar` on line 1.")],
        "(0..foo).bar\n        .qux { quux }\n",
    ),
    (
        &[1922],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n    each_pair do |_, suggestions|\n      suggestions.transform_values! { |v| v == 'true' }\n    end\n).to_h\n",
        &[(
            3,
            5,
            "Align `each_pair` with `refusal_advice_params.fetch(:actions).` on line 2.",
        )],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n           each_pair do |_, suggestions|\n             suggestions.transform_values! { |v| v == 'true' }\n           end\n).to_h\n",
    ),
    (
        &[1983, 2536],
        "add_to_git_repo(\n  initial_repo,\n  \"about.json\" =>\n    JSON\n        .parse(about_json(about_url: \"https://updated.site.com\"))\n        .tap { |h| h[:component] = true }\n        .to_json,\n)\n",
        &[
            (7, 9, "Align `.to_json` with `JSON` on line 4."),
            (6, 9, "Align `.tap` with `JSON` on line 4."),
            (5, 9, "Align `.parse` with `JSON` on line 4."),
        ],
        "add_to_git_repo(\n  initial_repo,\n  \"about.json\" =>\n    JSON\n    .parse(about_json(about_url: \"https://updated.site.com\"))\n    .tap { |h| h[:component] = true }\n    .to_json,\n)\n",
    ),
    (
        &[2027],
        "def run\n  (date_columns + candidate_columns).uniq\n                                      .select { |column_name| castable?(column_name) }\n                                      .each { |column_name| cast(column_name) }\nend\n",
        &[(3, 39, "Align `.select` with `.uniq` on line 2.")],
        "def run\n  (date_columns + candidate_columns).uniq\n                                    .select { |column_name| castable?(column_name) }\n                                      .each { |column_name| cast(column_name) }\nend\n",
    ),
    (
        &[1604],
        "User.a\n  .c\n  .b\n",
        &[
            (3, 3, "Align `.b` with `.a` on line 1."),
            (2, 3, "Align `.c` with `.a` on line 1."),
        ],
        "User.a\n    .c\n    .b\n",
    ),
    (
        &[2073],
        "@foo = Foo\n         .where(id: Bar.select(:id)\n           .joins(:bar)\n           .where.not(bar: { id: 123 }))\n",
        &[(2, 10, "Align `.where` with `Foo` on line 1.")],
        "@foo = Foo\n       .where(id: Bar.select(:id)\n           .joins(:bar)\n           .where.not(bar: { id: 123 }))\n",
    ),
    (
        &[2082],
        "@foo = Foo.\n         where(id: Bar.select(:id)\n           .joins(:bar))\n",
        &[(2, 10, "Align `where` with `Foo.` on line 1.")],
        "@foo = Foo.\n       where(id: Bar.select(:id)\n           .joins(:bar))\n",
    ),
    (
        &[2109],
        "@foo = Foo\n         &.where(id: Bar.select(:id)\n           &.joins(:bar))\n",
        &[(2, 10, "Align `&.where` with `Foo` on line 1.")],
        "@foo = Foo\n       &.where(id: Bar.select(:id)\n           &.joins(:bar))\n",
    ),
    (
        &[2172],
        "foo.bar do |x| x end\n.baz do |x| do_something(x) end\n",
        &[(2, 1, "Align `.baz` with `.bar` on line 1.")],
        "foo.bar do |x| x end\n   .baz do |x| do_something(x) end\n",
    ),
    (
        &[2185],
        "foo.bar\n.baz { |x|\n  do_something(x) }\n",
        &[(2, 1, "Align `.baz` with `.bar` on line 1.")],
        "foo.bar\n   .baz { |x|\n     do_something(x) }\n",
    ),
    (
        &[2200],
        "1 + a\n  .b\n  .c\n",
        &[
            (3, 3, "Align `.c` with `a` on line 1."),
            (2, 3, "Align `.b` with `a` on line 1."),
        ],
        "1 + a\n    .b\n    .c\n",
    ),
    (
        &[2343],
        "for n in a.\n  b\nend\n",
        &[(2, 3, "Align `b` with `a.` on line 1.")],
        "for n in a.\n         b\nend\n",
    ),
    (
        &[2358],
        "for n in a.\n    b\nend\n",
        &[(2, 5, "Align `b` with `a.` on line 1.")],
        "for n in a.\n         b\nend\n",
    ),
    (
        &[2400],
        "a.\n  b\nc.\n    d\n",
        &[(
            4,
            5,
            "Use 2 (not 4) spaces for indenting an expression spanning multiple lines.",
        )],
        "a.\n  b\nc.\n  d\n",
    ),
    (
        &[2475],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n      each_pair do |_, suggestions|\n        suggestions.transform_values! { |v| v == 'true' }\n      end\n).to_h\n",
        &[(
            3,
            7,
            "Align `each_pair` with `refusal_advice_params.fetch(:actions).` on line 2.",
        )],
        "parsed_params = refusal_advice_params.merge(\n  actions: refusal_advice_params.fetch(:actions).\n           each_pair do |_, suggestions|\n             suggestions.transform_values! { |v| v == 'true' }\n           end\n).to_h\n",
    ),
    // Held (finding kept, file unchanged).
    (
        &[2563],
        "method(key: value.foo.bar\n  .baz)\n",
        &[(2, 3, "Align `.baz` with `.foo` on line 1.")],
        "method(key: value.foo.bar\n  .baz)\n",
    ),
    (
        &[2571],
        "@foo = Foo\n  .where(id: Bar.select(:id)\n    .joins(:bar)\n    .where.not(bar: { id: 123 }))\n",
        &[(2, 3, "Align `.where` with `Foo` on line 1.")],
        "@foo = Foo\n       .where(id: Bar.select(:id)\n    .joins(:bar)\n    .where.not(bar: { id: 123 }))\n",
    ),
    (
        &[2580],
        "@foo = Foo.\n  where(id: Bar.select(:id)\n    .joins(:bar))\n",
        &[(2, 3, "Align `where` with `Foo.` on line 1.")],
        "@foo = Foo.\n       where(id: Bar.select(:id)\n    .joins(:bar))\n",
    ),
    (
        &[2607],
        "@foo = Foo\n  &.where(id: Bar.select(:id)\n    &.joins(:bar))\n",
        &[(2, 3, "Align `&.where` with `Foo` on line 1.")],
        "@foo = Foo\n       &.where(id: Bar.select(:id)\n    &.joins(:bar))\n",
    ),
    (
        &[2627],
        "User.a\n       .c\n       .b\n",
        &[
            (3, 8, "Align `.b` with `.a` on line 1."),
            (2, 8, "Align `.c` with `.a` on line 1."),
        ],
        "User.a\n    .c\n    .b\n",
    ),
    (
        &[2635],
        "1 + a\n       .b\n       .c\n",
        &[
            (3, 8, "Align `.c` with `a` on line 1."),
            (2, 8, "Align `.b` with `a` on line 1."),
        ],
        "1 + a\n    .b\n    .c\n",
    ),
    (
        &[2643],
        "if a.\n         b\n  something\nend\n",
        &[(2, 10, "Align `b` with `a.` on line 1.")],
        "if a.\n   b\n  something\nend\n",
    ),
    (
        &[2652],
        "formatted_int = int_part\n       .abs\n       .to_s\n       .reverse\n",
        &[
            (4, 8, "Align `.reverse` with `int_part` on line 1."),
            (3, 8, "Align `.to_s` with `int_part` on line 1."),
            (2, 8, "Align `.abs` with `int_part` on line 1."),
        ],
        "formatted_int = int_part\n                .abs\n                .to_s\n                .reverse\n",
    ),
    (
        &[2297],
        "if receiver.\n    nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `nil?` with `receiver.` on line 1.")],
        "if receiver.\n   nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2306],
        "if receiver\n  .nil? &&\n  !args.empty?\nend\n",
        &[(2, 3, "Align `.nil?` with `receiver` on line 1.")],
        "if receiver\n   .nil? &&\n  !args.empty?\nend\n",
    ),
    (
        &[2297],
        "unless receiver.\n    nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `nil?` with `receiver.` on line 1.")],
        "unless receiver.\n       nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2306],
        "unless receiver\n  .nil? &&\n  !args.empty?\nend\n",
        &[(2, 3, "Align `.nil?` with `receiver` on line 1.")],
        "unless receiver\n       .nil? &&\n  !args.empty?\nend\n",
    ),
    (
        &[2297],
        "while receiver.\n    nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `nil?` with `receiver.` on line 1.")],
        "while receiver.\n      nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2306],
        "while receiver\n  .nil? &&\n  !args.empty?\nend\n",
        &[(2, 3, "Align `.nil?` with `receiver` on line 1.")],
        "while receiver\n      .nil? &&\n  !args.empty?\nend\n",
    ),
    (
        &[2297],
        "until receiver.\n    nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `nil?` with `receiver.` on line 1.")],
        "until receiver.\n      nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2306],
        "until receiver\n  .nil? &&\n  !args.empty?\nend\n",
        &[(2, 3, "Align `.nil?` with `receiver` on line 1.")],
        "until receiver\n      .nil? &&\n  !args.empty?\nend\n",
    ),
    (
        &[2367],
        "a = int_part\n  .abs\n  .to_s\n  .reverse\n  .gsub(/...(?=.)/, '&_')\n  .reverse\n",
        &[
            (6, 3, "Align `.reverse` with `int_part` on line 1."),
            (5, 3, "Align `.gsub` with `int_part` on line 1."),
            (4, 3, "Align `.reverse` with `int_part` on line 1."),
            (3, 3, "Align `.to_s` with `int_part` on line 1."),
            (2, 3, "Align `.abs` with `int_part` on line 1."),
        ],
        "a = int_part\n    .abs\n    .to_s\n    .reverse\n    .gsub(/...(?=.)/, '&_')\n    .reverse\n",
    ),
    (
        &[2378],
        "a =\n  int_part\n    .abs\n    .to_s\n",
        &[
            (4, 5, "Align `.to_s` with `int_part` on line 2."),
            (3, 5, "Align `.abs` with `int_part` on line 2."),
        ],
        "a =\n  int_part\n  .abs\n  .to_s\n",
    ),
    (
        &[2387],
        "obj.a =\n  int_part\n    .abs\n    .to_s\n",
        &[
            (4, 5, "Align `.to_s` with `int_part` on line 2."),
            (3, 5, "Align `.abs` with `int_part` on line 2."),
        ],
        "obj.a =\n  int_part\n  .abs\n  .to_s\n",
    ),
    (
        &[2367],
        "a[:key] = int_part\n  .abs\n  .to_s\n  .reverse\n  .gsub(/...(?=.)/, '&_')\n  .reverse\n",
        &[
            (6, 3, "Align `.reverse` with `int_part` on line 1."),
            (5, 3, "Align `.gsub` with `int_part` on line 1."),
            (4, 3, "Align `.reverse` with `int_part` on line 1."),
            (3, 3, "Align `.to_s` with `int_part` on line 1."),
            (2, 3, "Align `.abs` with `int_part` on line 1."),
        ],
        "a[:key] = int_part\n          .abs\n          .to_s\n          .reverse\n          .gsub(/...(?=.)/, '&_')\n          .reverse\n",
    ),
    (
        &[2378],
        "a[:key] =\n  int_part\n    .abs\n    .to_s\n",
        &[
            (4, 5, "Align `.to_s` with `int_part` on line 2."),
            (3, 5, "Align `.abs` with `int_part` on line 2."),
        ],
        "a[:key] =\n  int_part\n  .abs\n  .to_s\n",
    ),
    (
        &[2387],
        "obj.a[:key] =\n  int_part\n    .abs\n    .to_s\n",
        &[
            (4, 5, "Align `.to_s` with `int_part` on line 2."),
            (3, 5, "Align `.abs` with `int_part` on line 2."),
        ],
        "obj.a[:key] =\n  int_part\n  .abs\n  .to_s\n",
    ),
    (
        &[2671],
        "if receiver.\n         nil? &&\n         !args.empty?\nend\n",
        &[(2, 10, "Align `nil?` with `receiver.` on line 1.")],
        "if receiver.\n   nil? &&\n         !args.empty?\nend\n",
    ),
    (
        &[2680],
        "if receiver\n    .nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `.nil?` with `receiver` on line 1.")],
        "if receiver\n   .nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2697],
        "if a\n  something.\n         something_else\nend\n",
        &[(
            3,
            10,
            "Use 2 (not 7) spaces for indenting an expression spanning multiple lines.",
        )],
        "if a\n  something.\n    something_else\nend\n",
    ),
    (
        &[2671],
        "unless receiver.\n         nil? &&\n         !args.empty?\nend\n",
        &[(2, 10, "Align `nil?` with `receiver.` on line 1.")],
        "unless receiver.\n       nil? &&\n         !args.empty?\nend\n",
    ),
    (
        &[2680],
        "unless receiver\n    .nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `.nil?` with `receiver` on line 1.")],
        "unless receiver\n       .nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2697],
        "unless a\n  something.\n         something_else\nend\n",
        &[(
            3,
            10,
            "Use 2 (not 7) spaces for indenting an expression spanning multiple lines.",
        )],
        "unless a\n  something.\n    something_else\nend\n",
    ),
    (
        &[2671],
        "while receiver.\n         nil? &&\n         !args.empty?\nend\n",
        &[(2, 10, "Align `nil?` with `receiver.` on line 1.")],
        "while receiver.\n      nil? &&\n         !args.empty?\nend\n",
    ),
    (
        &[2680],
        "while receiver\n    .nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `.nil?` with `receiver` on line 1.")],
        "while receiver\n      .nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2697],
        "while a\n  something.\n         something_else\nend\n",
        &[(
            3,
            10,
            "Use 2 (not 7) spaces for indenting an expression spanning multiple lines.",
        )],
        "while a\n  something.\n    something_else\nend\n",
    ),
    (
        &[2671],
        "until receiver.\n         nil? &&\n         !args.empty?\nend\n",
        &[(2, 10, "Align `nil?` with `receiver.` on line 1.")],
        "until receiver.\n      nil? &&\n         !args.empty?\nend\n",
    ),
    (
        &[2680],
        "until receiver\n    .nil? &&\n    !args.empty?\nend\n",
        &[(2, 5, "Align `.nil?` with `receiver` on line 1.")],
        "until receiver\n      .nil? &&\n    !args.empty?\nend\n",
    ),
    (
        &[2697],
        "until a\n  something.\n         something_else\nend\n",
        &[(
            3,
            10,
            "Use 2 (not 7) spaces for indenting an expression spanning multiple lines.",
        )],
        "until a\n  something.\n    something_else\nend\n",
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

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (lines, source, expected, after) in REPORTED {
        let mut expected: Vec<_> = expected
            .iter()
            .map(|(line, col, message)| (*line, *col, (*message).to_owned()))
            .collect();
        expected.sort();
        assert_eq!(findings(source), expected, "spec lines {lines:?}: {source}");
        assert_correction(&MultilineMethodCallIndentation, source, after);
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
fn literal_text_comments_and_observers_hold_or_keep_bytes() {
    // A heredoc body inside the shifted block keeps its bytes.
    let source = "a\n    .each do\n      x = <<~E\n        body\n      E\n    end\n";
    assert_correction(
        &MultilineMethodCallIndentation,
        source,
        "a\n  .each do\n    x = <<~E\n        body\n      E\n  end\n",
    );
    // A document comment in the shifted range holds the edit.
    let source = "a\n    .each do\n=begin\n=end\n    end\n";
    let found = correction_offenses(&MultilineMethodCallIndentation, source);
    assert_eq!(found.len(), 1);
    assert!(found[0].correction.is_none());
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\npayload",
    ] {
        let source = format!("a\n.b\n{observer}\n");
        let found = correction_offenses(&MultilineMethodCallIndentation, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
    assert_correction(
        &MultilineMethodCallIndentation,
        "a\n.b\np File.read('example.rb')\n",
        "a\n  .b\np File.read('example.rb')\n",
    );
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
