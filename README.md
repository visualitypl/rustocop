# rustocop

A fast, opinionated Ruby linter written in Rust.

rustocop ships all 613 standard RuboCop cops. They behave like their RuboCop
counterparts, but rustocop makes the style decisions for you, needs no Ruby installation to run, and
lints RuboCop's own 1,800 source files in about half a second.

```
$ rustocop app
app/order.rb:3:5: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)
app/order.rb:4:15: C: Pass `&:price` as an argument to `map` instead of a block. (Style/SymbolProc)
app/order.rb:8:5: C: Prefer double-quoted strings unless you need single quotes to avoid extra backslashes for escaping. (Style/StringLiterals)
app/order.rb:8:17: C: Favor `format` over `String#%`. (Style/FormatString)

1 file inspected, 4 offenses detected
```

- **Zero config.** Sensible defaults out of the box. Add `rustocop.toml` only
  to exclude paths or switch cops off.
- **No Ruby runtime.** The binary needs neither Ruby, Bundler nor gems.
- **Fast.** Parses with [Prism](https://github.com/ruby/prism) and lints files
  in parallel across all cores.
- **Safe autocorrection.** `--fix` applies only edits that provably keep the
  program's behaviour. Everything else stays a report.
- **Not a RuboCop drop-in.** Cop names and `# rubocop:disable` comments are
  familiar, but rustocop does not read `.rubocop.yml` and has few
  settings: limits, and single or double quotes. `--import-rubocop-config`
  carries over the settings that have a counterpart. The defaults are the
  product.

## Status

Version 1.2.0.

| Department | Cops |
| --- | ---: |
| Style | 299 |
| Lint | 160 |
| Layout | 100 |
| Naming | 19 |
| Gemspec | 10 |
| Metrics | 10 |
| Bundler | 7 |
| Security | 7 |
| Migration | 1 |

Each cop matches the behaviour of its RuboCop 1.92.0 counterpart at
rustocop's fixed settings and is verified against RuboCop's own spec examples
(see [Development](#development)). Most cops are on by default; 35 Style
cops, the ten Metrics cops, 14 Layout cops (the line-break cops, `Layout/ClassStructure`,
`Layout/SingleLineBlockChain`, `Layout/MultilineAssignmentLayout`,
`Layout/EmptyLineAfterMultilineCondition` and
`Layout/HeredocArgumentClosingParenthesis`), five Lint cops
(`Lint/ConstantResolution`, `Lint/HeredocMethodCallPosition`,
`Lint/NumberConversion`, `Lint/ShadowingOuterLocalVariable`,
`Lint/UnusedPrivateMethod`), `Bundler/GemComment`, `Bundler/GemVersion`,
`Gemspec/DependencyVersion`, `Naming/InclusiveLanguage` and
`Style/FrozenStringLiteralComment` are opt-in. Three of these choices are
rustocop's own, where RuboCop has the cop on: the Metrics cops, because the
right limits differ from one codebase to the next, `Style/Documentation` and
`Style/FrozenStringLiteralComment`. `[cops.Metrics]` with `enabled = true`
switches the department on, and a `max` for a Metrics cop switches that cop
on. While one of these cops is off, a
`# rubocop:disable` comment for it is not reported as unnecessary, because
RuboCop may still need the comment. A few cops apply only to some
Ruby versions. `Style/ReverseFind` runs only when `ruby_version` is `"4.0"`.
`Security/YAMLLoad`, `Lint/ItWithoutArgumentsInBlock`,
`Lint/UselessElseWithoutRescue` and `Lint/NonDeterministicRequireOrder` apply
only to Ruby versions older than 3.4; they stay registered but report nothing.

## How it differs from RuboCop

- **One style.** Where RuboCop offers a choice (`EnforcedStyle`, thresholds,
  allow-lists), rustocop picks one and does not expose the setting. See
  [Fixed defaults](#fixed-defaults).
- **Ruby 3.4 or 4.0.** Source is parsed with Ruby 3.4 grammar by default,
  including implicit `it`, argument forwarding and encoding comments. Ruby 4
  only syntax is then a parse error. A project that declares Ruby 4.0 is
  parsed with Ruby 4.0 grammar. Older Ruby versions are checked as Ruby 3.4.
  See [Ruby version](#ruby-version).
- **File-local analysis.** Every file is linted independently and in
  parallel. Cops that need cross-file state in RuboCop (for example
  `Lint/DuplicateMethods` across files) work within one file here.
- **Standard cops only.** All nine RuboCop departments are covered. Metrics
  cops are opt-in, and `max` is their only setting. There is no support for
  extension gems such as rubocop-rails or rubocop-rspec.
- **No unsafe autocorrect, formatter or server mode.**

## Installation

The [releases page](https://github.com/visualitypl/rustocop/releases) has
binaries for Linux (x86_64 and aarch64, glibc 2.35 or newer) and macOS (Intel
and Apple silicon). Each archive holds the `rustocop` executable; put it in a
directory on your `PATH`.

Building from source needs Rust 1.99 or newer, a C compiler for the vendored
Prism parser, and libclang.

```sh
git clone git@github.com:visualitypl/rustocop.git
cd rustocop
cargo install --path .        # or: cargo build --release
rustocop --version
```

## Usage

```sh
rustocop                           # lint the current directory
rustocop app lib                   # lint specific paths
rustocop --only Lint               # one department or cop (comma-separated)
rustocop --only Style/IpAddresses  # force-enable an opt-in cop
rustocop --except Lint/Debugger
rustocop --format github           # GitHub Actions annotations
rustocop --format offenses         # the number of offenses per cop
rustocop --format json             # RuboCop's JSON, for editor plugins
rustocop --write-baseline          # silence today's offenses in rustocop.toml
rustocop --import-rubocop-config   # write a rustocop.toml from .rubocop.yml
rustocop --fix app lib             # apply safe corrections in place
rustocop --fix --fix-passes 1 app  # apply a single correction pass
cat file.rb | rustocop --stdin file.rb
rustocop --show-ruby-version       # the Ruby version of the run and its source
```

Directories are searched for `.rb` files, `Gemfile`, `*.gemfile` and
`*.gemspec`, honouring `.gitignore`. Other files (`Rakefile`, `*.rake`, `bin/`
scripts) are linted when named explicitly, for example `rustocop lib Rakefile`. `db/schema.rb`, `vendor/**` and
`node_modules/**` are excluded by default.

Exit codes: **0** clean, **1** offenses remain, **2** errors (configuration,
unreadable or unparseable files, internal cop errors). Other files continue
after a file error, and other cops continue after a cop crash. Lint and
Security offenses are reported as errors (`E`); other departments report
conventions (`C`).

In GitHub Actions, `--format github` turns every offense into an inline
annotation:

```yaml
- run: rustocop --format github
```

### Editors

`--format json` prints the report in the schema of RuboCop's `--format json`,
so an editor plugin written for RuboCop can run rustocop when you name it as
the executable. rustocop also accepts the `--force-exclusion` and `--server`
options that those plugins pass, and ignores them: the excludes of the config
always apply, and there is no server.

```lua
-- Neovim, nvim-lint
require("lint").linters.rubocop.cmd = "rustocop"
```

```vim
" Vim, ALE
let g:ale_ruby_rubocop_executable = 'rustocop'
```

The output differs from RuboCop's in these ways:

- An offense records where it starts and not where it ends. Each `location`
  is the one character at the start, so an editor marks that character and
  not the whole expression.
- `correctable` says that the cop has safe corrections, not that this
  offense has one. `corrected` is always `false`.
- `metadata` holds `rustocop_version` only.
- A syntax error is an offense of `Lint/Syntax` with the severity `fatal`, as
  in RuboCop. A file that cannot be read, and an internal error, is a string
  in the `errors` of its file.

## Adopting rustocop in an existing codebase

A first run on code that was written to another style reports a lot. Three
steps make it manageable. The output below is from RuboCop's own source.

1. See which cops report the most:

   ```
   $ rustocop --format offenses
   53363  Style/StringLiterals
     933  Style/FormatStringToken
     483  Layout/LineLength
   ...

   1793 files inspected, 56152 offenses detected
   ```

2. Let `rustocop --fix` correct what is safe to correct:

   ```
   $ rustocop --fix
   ...
   1793 files inspected, 39333 corrections applied, 16796 offenses remaining
   ```

3. Write a baseline for the rest:

   ```
   $ rustocop --write-baseline
   Wrote /home/me/app/rustocop.toml: 16798 offenses silenced, 11 cops switched off, 30 cops with skipped files.
   ```

   The baseline is a `rustocop.toml` with one entry per cop that has
   offenses. A cop with offenses in at most 15 files gets an `exclude` list
   of those files, so it still checks every other file and every new one. A
   cop with offenses in more files is switched off with `enabled = false`.
   A run with the baseline reports nothing, and each entry that you delete
   brings its offenses back for you to fix.

`--write-baseline` lints the given paths, or the current directory. When a
`rustocop.toml` already exists, the run uses it and the baseline is added to
it: a cop that has a table there is switched off or gets more `exclude`
paths in that table, and any other cop gets a new table at the end of the
file. Everything else in the file stays as it is, comments included. File
errors, such
as syntax errors, are listed and stay errors, because no config silences
them. The option cannot be combined with `--fix`, `--stdin`, `--only` or
`--except`.

### Coming from RuboCop

`rustocop --import-rubocop-config` reads the `.rubocop.yml` in the current
directory, and the local files that it names in `inherit_from`, such as
`.rubocop_todo.yml`. It writes a `rustocop.toml` next to it and lists the
settings that it does not carry over. This is the output for RuboCop's own
config, shortened:

```
$ rustocop --import-rubocop-config
Wrote rustocop.toml from .rubocop_todo.yml and .rubocop.yml.
Not carried over; rustocop has no setting for these:
  plugins
  AllCops: NewCops, SuggestExtensions, TargetRubyVersion, UseProjectIndex
  Layout/HashAlignment: EnforcedColonStyle, EnforcedHashRocketStyle
  Layout/LineLength: AllowedPatterns
  not rustocop cops or departments: RSpec, RSpec/AnyInstance, Rake/MethodDefinitionInTask
```

The import carries over:

- `AllCops: Exclude`, as the top-level `exclude`;
- `AllCops: TargetRubyVersion` when it is 3.4 or 4.0, as `ruby_version`;
- `Enabled: true` and `Enabled: false` of a cop or a department;
- `Exclude` of a cop;
- `Max` of `Layout/LineLength` and of a Metrics cop, when it is a whole
  number;
- `EnforcedStyle` of `Style/StringLiterals`,
  `Style/StringLiteralsInInterpolation` and `Style/QuotedSymbols`;
- `AllowedPatterns` of `Layout/LineLength`. A pattern is a Ruby regexp, and
  the import keeps the ones that the `regex` crate reads too. It lists the
  others, such as a pattern with a lookbehind.

RuboCop's default for string literals is single quotes and rustocop's is
double quotes. A config that names no quote style means RuboCop's default,
so the import writes `enforced_style = "single_quotes"` for the two string
cops. It does not when the config inherits from a gem or a URL, because it
cannot read what those say. On RuboCop's own source, a first run reports
56,152 offenses without a config and 1,053 with the imported one.

It lists everything else: other style parameters, cops that rustocop does not
have, `require`, `plugins`, `inherit_gem`, an `inherit_from` URL, and
`Exclude` entries that are absolute paths or regexps. A setting in
`.rubocop.yml` replaces the same setting of an inherited file, as in RuboCop;
`inherit_mode` with `merge: [Exclude]` at the top of a file joins the
`Exclude` lists.

The import never changes an existing config: when a `rustocop.toml` already
exists, it stops with an error. Run it first, and then `--write-baseline` to
silence what rustocop still reports with the imported config.

## Fixed defaults

| Setting | rustocop |
| --- | --- |
| String literals | double quotes unless `enforced_style` says `"single_quotes"` (see [Configuration](#configuration)) |
| Hash syntax | `a: 1`; shorthand `{x:}` and `{x: x}` both accepted |
| Raising exceptions | `raise`, never `fail` |
| Line length | 120 columns unless `max` sets another (see [Configuration](#configuration)) |
| `# frozen_string_literal: true` | not required unless the cop is enabled |
| Documentation comments | not required unless `Style/Documentation` is enabled |
| Metrics (method length, complexity, ...) | not checked unless the cop is enabled; RuboCop's limits unless `max` sets another |

The remaining defaults follow RuboCop 1.92.0. Pin the binary version if new
default-on cops are unwanted.

## Safe corrections

`--fix` applies only corrections that are safe by construction: the edit
preserves bindings, evaluation order, control flow, literal values, encoding
and comments. Offenses whose fix is unsafe, and individual edits the
implementation cannot prove safe, stay reported but untouched. Without
`--fix`, files are never written.

```
$ rustocop --fix app
...
1 file inspected, 1 correction applied, 4 offenses remaining
```

Corrections are applied in passes. Each pass applies its edits to the
source, then lints the result again, so the next pass sees accurate positions
and any corrections the last one made possible. Overlapping edits are
resolved deterministically; the losing edit waits for the next pass. The
passes repeat in memory until one changes nothing, and the file is written
once.

Two guards stop a file whose corrections do not settle: a pass that restores
an earlier version of the file, and a limit of 200 passes. Either one is
reported as an error for that file, which keeps the last validated version.
`--fix-passes N` stops after N passes without an error; `--fix-passes 1`
applies exactly one pass.

Corrected source is validated before an atomic replacement that preserves
permissions and symlinks. If the first pass fails validation, the original
is left unchanged; if a later pass does, the earlier passes are kept and the
failure is reported. Disabled lines, path exclusions and files with syntax
errors are never rewritten. `--fix` cannot be combined with `--stdin`.

## Configuration

An optional `rustocop.toml` is discovered upward from the current directory to
the git root. Paths are relative to the file.

```toml
exclude = ["bin/**", "spec/fixtures/**"]

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

Validation is strict: unknown keys, unknown cops or departments, invalid
globs and unsupported Ruby versions are errors before any linting. Department tables accept only `enabled`.

`max` is a whole number of at least 1, the `Max` of the same cop in RuboCop.
`Layout/LineLength` accepts it, and so do nine Metrics cops: `AbcSize`,
`BlockLength`, `BlockNesting`, `ClassLength`, `CyclomaticComplexity`,
`MethodLength`, `ModuleLength`, `ParameterLists` and `PerceivedComplexity`.
The Metrics cops are off by default, so a `max` for one of them also switches
it on, unless its table says `enabled = false`. Every other parameter of these
cops is fixed at RuboCop's default.

`enforced_style` is the `EnforcedStyle` of the same cop in RuboCop. Three cops
accept it, with `"double_quotes"` as the default:

- `Style/StringLiterals`: `"double_quotes"` or `"single_quotes"`.
- `Style/StringLiteralsInInterpolation`: the same two values, for the strings
  inside `#{}`. It does not follow `Style/StringLiterals`.
- `Style/QuotedSymbols`: the same two values, or
  `"same_as_string_literals"`. Without a setting it has the style of
  `Style/StringLiterals`.

`allowed_patterns` is the `AllowedPatterns` of `Layout/LineLength` in RuboCop:
a list of regular expressions, and a line longer than `max` is accepted when
one of them matches it. The syntax is that of Rust's
[`regex`](https://docs.rs/regex/latest/regex/#syntax) crate, which has no
lookaround and no backreferences; a pattern that it cannot read is an error.
`\d`, `\w` and `\s` also match non-ASCII characters, which they do not in
Ruby.

Selection precedence, highest first: `--except`, `--only` (which also
force-enables opt-in cops), per-cop `enabled`, department `enabled`, the
cop's default. `--only Style` enables the opt-in Style cops too, including
ones that hold competing opinions. Unknown selectors are errors with a
did-you-mean suggestion where possible.

### Ruby version

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

Outside a git repository, rustocop reads the first directory only. A file
that names no version, or another Ruby implementation such as JRuby, is
skipped. A version older than 3.4 counts as 3.4, and a version newer than 4.0
counts as 4.0. When no file names a version, the version is 3.4.

`rustocop --show-ruby-version` prints the version of the run and where it
comes from, and lints nothing:

```console
$ rustocop --show-ruby-version
Ruby 4.0 (/home/me/app/.ruby-version declares 4.0.9)
```

The version selects three things:

- the grammar. With `"4.0"`, a line can start with `&&` or `||`, and
  `private def name = command argument` is valid. With `"3.4"`, both are
  syntax errors, as in Ruby 3.4.
- the cops that apply. `Style/ReverseFind` runs only with `"4.0"`, and
  `Lint/RedundantRequireStatement` then also reports `require 'pathname'`.
- the check of corrected code. `--fix` validates each correction with the
  same grammar.

The Ruby 4.0 grammar is compared with Ruby 4.0.9 in the tests. The tests that
run code before and after a correction run on Ruby 3.4.1 and again on Ruby
4.0.9.

Cops with a fixed path policy (for example `Style/FrozenStringLiteralComment`
skipping `*.arb` files) keep it even under `--only`, and the policy applies to
`--stdin PATH` as well.

### Inline suppression

```ruby
binding.pry # rustocop:disable Lint/Debugger -- this line only

# rustocop:disable-next Lint/Debugger
binding.pry # next statement, including all of its lines and heredocs

# rustocop:disable Lint/Debugger
binding.pry
# rustocop:enable Lint/Debugger
```

Both `rustocop:` and `rubocop:` prefixes work. Targets may be full cop names,
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

## Development

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
tools/check-msrv.sh                       # builds and tests on Rust 1.99
cargo run -- path/to/file.rb
echo 'binding.pry' | cargo run -- --stdin x.rb
cargo run --example list_cops             # every registered cop name
```

All three of fmt, clippy and test must be clean before a commit.

**Test layout.** Every file in `tests/` is a module of one test program,
`integration`. `tests/main.rs` lists the modules, so add a line there for a
new test file; a test fails when a file is missing from the list. Shared
helpers are in `tests/support/`. Run the tests of one file by its module name:

```sh
cargo test --test integration cli::
```

**Tests in Docker.** The tests assume one environment: Ubuntu 24.04, its
system Ruby 3.2 at `/usr/bin/ruby`, Ruby 3.4.1 and Ruby 4.0.9 built against
the system OpenSSL, and the RuboCop checkout at the pinned revision.
`tools/docker-test.sh` builds an image with exactly that and runs fmt, clippy
and the tests in it. It gives the same result on every machine, and the
oracle always runs there. Pass a command to run something else:

```sh
tools/docker-test.sh                      # fmt, clippy and all tests
tools/docker-test.sh cargo test --lib     # any other command
```

The tests run in parallel through [cargo-nextest](https://nexte.st), in two
passes. The first pass runs every test, and the tests that start a Ruby
program use Ruby 3.4.1. The second pass sets `RUSTOCOP_TEST_RUBY=4.0` and
runs the integration tests again with Ruby 4.0.9. A test that is valid on
Ruby 3.4 only starts with `if crate::support::runtime::skips("reason")
{ return; }`, so the second pass skips it and the reason stays in the code.

A full run needs Docker with 8 GB of memory and about 4 GB of disk. It takes
about 15 minutes the first time and about 13 minutes once the build output
exists. The source is mounted
read-only. Build output stays in the Docker volume `rustocop-test-target`
(about 1.4 GB); remove it with
`docker volume rm rustocop-test-target`.

**Native runs.** Many tests run `ruby` from `PATH` and compare its output, so
they need Ruby 3.2 or 3.3. `mise.toml` selects Ruby 3.2.6. With
[mise](https://mise.jdx.dev) installed, set it up once and run cargo through
it, in a shell with a UTF-8 locale:

```sh
mise install
mise exec -- cargo test
```

A native run covers less than the Docker run. The checks against Ruby 3.4.1
run only when `RUSTOCOP_RUBY34` names its executable, and they need the Linux
tools `timeout` and `prlimit`. The checks against Ruby 4.0 run only when
`RUSTOCOP_RUBY40` names its executable. `RUSTOCOP_TEST_RUBY=4.0` makes the
integration tests use that executable in place of the Ruby 3.4 one. On macOS some tests fail, because
`/usr/bin/ruby` is Ruby 2.6 there and the temporary directory sits behind a
symlink.

Every cop lives in its own file under `src/cops/<department>/`, implements the
`Cop` trait and is registered in `Registry::new()` in `src/cops/mod.rs`.
`src/cops/lint/debugger.rs` is the reference implementation. Tests sit next
to the cop and in `tests/`; where semantics match RuboCop, its spec examples
are ported (RuboCop is MIT licensed and the ported modules keep the
attribution).

**RuboCop spec oracle.** `tools/oracle` runs RuboCop's own spec examples for
every registered cop against the binary and reports misses and false
positives by line and column. `cargo test` runs it when Ruby, RuboCop's
runtime gems and a checkout at the pinned revision are available, and skips
otherwise. Known deviations are listed with reasons in
`tools/oracle/baseline.txt`. See [tools/oracle/README.md](tools/oracle/README.md).

**Differential check.** `tools/differential.rb` compares rustocop with a local
RuboCop executable over a directory of real Ruby code and lists every
position where they disagree. It is an optional local check, never run in
CI, and never executes the corpus.

## Performance

Linting RuboCop's own source (revision `97f30c4`: 1,793 Ruby files, about
390,000 lines) with the default cops:

| Command                               | Mean ± σ        |
| ------------------------------------- | --------------- |
| `rustocop .`                          | 371 ms ± 43 ms  |
| `RAYON_NUM_THREADS=1 rustocop .`      | 2.39 s ± 0.02 s |
| `rubocop --parallel`, empty cache     | 6.57 s ± 0.16 s |
| `rubocop --parallel`, warm cache      | 775 ms ± 10 ms  |

That is roughly 4,800 files or 1,000,000 lines per second. rustocop keeps no
result cache, so every run is a full lint.

Measured with hyperfine 1.18 (50 runs for rustocop, 5 and 10 for RuboCop,
files already in the page cache) on an Apple M5 Pro with 15 cores, under
Linux aarch64 in the `Dockerfile.test` image: a `--release` build of rustocop
1.0.0 with Rust 1.99.1, and RuboCop 1.92.0 on Ruby 3.2.3 with
`NewCops: enable` and no extensions. The RuboCop rows are for scale, not a
like-for-like race: the defaults differ, and rustocop reports 56,152 offenses
on this code where RuboCop reports 5,970.

## License

MIT. Not affiliated with RuboCop.
