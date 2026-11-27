# Configuration

- [rustocop.toml](#rustocoptoml)
- [Settings](#settings)
- [Cop selection](#cop-selection)
- [Ruby version](#ruby-version)
- [Inline directives](#inline-directives)

## rustocop.toml

rustocop needs no configuration. An optional `rustocop.toml` is discovered
upward from the current directory to the git root. Paths in it are relative to
the file.

```toml
exclude = ["bin/**", "spec/fixtures/**"]
ruby_version = "4.0"

[cops."Style"]
enabled = false

[cops."Lint/Debugger"]
exclude = ["spec/support/**"]

[cops."Layout/LineLength"]
max = 100
allowed_patterns = ['^\s*#', '\A +(it|describe) ']

[cops."Metrics/MethodLength"]
max = 25

[cops."Style/StringLiterals"]
enforced_style = "single_quotes"
```

Validation is strict: unknown keys, unknown cops or departments, invalid globs
and unsupported Ruby versions are errors before any linting. Department tables
accept only `enabled`. Every cop table accepts `enabled`, `exclude` and
`include`. `include` is RuboCop's `Include` of a cop: the cop checks only the
files that one of its globs matches, as in

```toml
[cops."Style/StringLiterals"]
include = ["app/**/*", "config/**/*", "lib/**/*", "test/**/*", "Gemfile"]
```

It replaces the paths a cop is fixed to (such as the Gemfiles of the Bundler
cops), but not the paths it is fixed to skip.

## Settings

A setting has the name of the RuboCop parameter in snake case, and its
meaning. `cargo run --example list_cops -- --settings` prints every setting
the cops declare.

### `max`

A whole number of at least 1, the `Max` of the same cop in RuboCop.
`Layout/LineLength` accepts it, and so do nine Metrics cops: `AbcSize`,
`BlockLength`, `BlockNesting`, `ClassLength`, `CyclomaticComplexity`,
`MethodLength`, `ModuleLength`, `ParameterLists` and `PerceivedComplexity`.
The Metrics cops are off by default, so a `max` for one of them also switches
it on, unless its table says `enabled = false`. Every other parameter of these
cops is fixed at RuboCop's default.

### `enforced_style`

The `EnforcedStyle` of the same cop in RuboCop.

- `Layout/CaseIndentation`: `"case"` (the default) or `"end"`, the keyword
  that `when` and `in` are as deep as.
- `Layout/SpaceInsideArrayLiteralBrackets`: `"no_space"` (the default),
  `"space"` or `"compact"`. `Layout/FirstArrayElementLineBreak` and
  `Layout/LineLength` read `"compact"`, as in RuboCop.
- `Style/HashSyntax`: `"ruby19"` (the default), `"hash_rockets"`,
  `"no_mixed_keys"` or `"ruby19_no_mixed_keys"`.
- `Style/StringLiterals`: `"double_quotes"` (the default) or
  `"single_quotes"`. `Style/EmptyLiteral`, `Style/EmptyHeredoc` and
  `Style/RedundantRegexpArgument` write a string in the quotes of this style,
  and in single quotes when this cop is disabled, as in RuboCop.
- `Style/StringLiteralsInInterpolation`: the same two values, for the strings
  inside `#{}`. It does not follow `Style/StringLiterals`.
- `Style/QuotedSymbols`: the same two values, or `"same_as_string_literals"`.
  Without a setting it has the style of `Style/StringLiterals`, and single
  quotes when that cop is disabled.

### `enforced_style_align_with`

The `EnforcedStyleAlignWith` of `Layout/EndAlignment`: `"keyword"` (the
default), `"variable"` or `"start_of_line"`. As in RuboCop,
`Layout/ElseAlignment`, `Layout/IndentationWidth` and the correction of
`Style/ConditionalAssignment` follow it, even while `Layout/EndAlignment` is
disabled.

### `enforced_style_for_empty_brackets` and `enforced_style_for_empty_braces`

The spacing inside empty brackets of `Layout/SpaceInsideArrayLiteralBrackets`
and inside empty braces of `Layout/SpaceInsideBlockBraces`: `"no_space"` (the
default) or `"space"`.

### `allow_multiple_return_values` and `allow_as_expression_separator`

`true` or `false` (the default). `Style/RedundantReturn` accepts a `return`
of several values (`return 1, 2`) with `allow_multiple_return_values`;
`Style/Semicolon` accepts a semicolon between two expressions on a line
(`x = 1; y = 2`) with `allow_as_expression_separator`.

### `allowed_patterns`

The `AllowedPatterns` of `Layout/LineLength`: a list of regular expressions,
and a line longer than `max` is accepted when one of them matches it. The
syntax is that of Rust's [`regex`](https://docs.rs/regex/latest/regex/#syntax)
crate, which has no lookaround and no backreferences; a pattern that it cannot
read is an error. `\d`, `\w` and `\s` also match non-ASCII characters, which
they do not in Ruby.

## Cop selection

Precedence, highest first:

1. `--except`
2. `--only`, which also force-enables opt-in cops
3. per-cop `enabled`
4. department `enabled`
5. the cop's default

A department in `--only`, or `enabled = true` on a department, also enables
its opt-in cops, including ones that hold competing opinions. As in RuboCop,
13 opt-in Layout cops (all but `Layout/HeredocArgumentClosingParenthesis`)
are enabled only by their own name. Unknown selectors are errors with a
did-you-mean suggestion where possible.

The opt-in cops are listed in the
[migration guide](migrating-from-rubocop.md#opt-in-cops). While one of them is
off, a `# rubocop:disable` comment for it is not reported as unnecessary,
because RuboCop may still need the comment.

Cops with a fixed path policy (for example `Style/FrozenStringLiteralComment`
skipping `*.arb` files) keep it even under `--only`, and the policy applies to
`--stdin PATH` as well.

## Ruby version

```toml
ruby_version = "4.0"
```

`ruby_version` names the Ruby version that the code runs on. The values are
`"3.4"` and `"4.0"`. Without this key, rustocop reads the version from the
project. It looks at these files in this order, first in the directory of
`rustocop.toml` (or the current directory when there is no config file), then
in each parent directory up to the git root:

1. `.ruby-version`
2. `.tool-versions`
3. `mise.toml`, then `.mise.toml`
4. `Gemfile.lock`, then `gems.locked` (the `RUBY VERSION` section)

Outside a git repository, rustocop reads the first directory only. A file that
names no version, or another Ruby implementation such as JRuby, is skipped. A
version older than 3.4 counts as 3.4, and a version newer than 4.0 counts as
4.0. When no file names a version, the version is 3.4.

`rustocop --show-ruby-version` prints the version of the run and where it
comes from, and lints nothing:

```console
$ rustocop --show-ruby-version
Ruby 4.0 (/home/me/app/.ruby-version declares 4.0.9)
```

The version selects three things:

- **The grammar.** With `"4.0"`, a line can start with `&&` or `||`, and
  `private def name = command argument` is valid. With `"3.4"`, both are
  syntax errors, as in Ruby 3.4.
- **The cops that apply.** `Style/ReverseFind` runs only with `"4.0"`, and
  `Lint/RedundantRequireStatement` then also reports `require 'pathname'`.
  `Security/YAMLLoad`, `Lint/ItWithoutArgumentsInBlock`,
  `Lint/UselessElseWithoutRescue` and `Lint/NonDeterministicRequireOrder`
  apply only to Ruby versions older than 3.4, so they report nothing.
- **The check of corrected code.** Corrections are validated with the same
  grammar.

## Inline directives

```ruby
binding.pry # rubocop:disable Lint/Debugger -- this line only

# rubocop:disable-next Lint/Debugger
binding.pry # next statement, including all of its lines and heredocs

# rubocop:disable Lint/Debugger
binding.pry
# rubocop:enable Lint/Debugger
```

Both `rubocop:` and `rustocop:` prefixes work. Targets may be full cop names,
names without the department, departments or `all`. `todo` is an alias for
`disable` and `todo-next` for `disable-next`.

- `enable-next Cop` suspends an open disable for the next statement.
- `next -Cop +Other` disables one cop and enables another for the next
  statement.
- `push [+Cop -Other]` saves the open disable state and `pop` restores it.
  Pushes nest; an unmatched `pop` does nothing.

Source `enable` directives can switch on opt-in cops within their scope.
`--only`, `--except`, path policies and exclusions still take precedence. A
multi-line offense is suppressed if any of its highlighted lines is disabled.
