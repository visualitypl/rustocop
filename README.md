# rustocop

A fast, opinionated Ruby linter written in Rust.

rustocop ships all 613 standard RuboCop cops. They behave like their RuboCop
counterparts, but rustocop makes the style decisions for you, needs no Ruby installation to run, and
lints thousands of files in a fraction of a second.

```
$ rustocop app
app/order.rb:1:1: C: Missing top-level documentation comment for `class Order`. (Style/Documentation)
app/order.rb:3:5: E: Remove debugger entry point `binding.pry`. (Lint/Debugger)
app/order.rb:4:15: C: Pass `&:price` as an argument to `map` instead of a block. (Style/SymbolProc)
app/order.rb:8:5: C: Prefer double-quoted strings unless you need single quotes to avoid extra backslashes for escaping. (Style/StringLiterals)
app/order.rb:8:17: C: Favor `format` over `String#%`. (Style/FormatString)

1 file inspected, 5 offenses detected
```

- **Zero config.** Sensible defaults out of the box. Add `rustocop.toml` only
  to exclude paths or switch cops off.
- **No Ruby runtime.** The binary needs neither Ruby, Bundler nor gems.
- **Fast.** Parses with [Prism](https://github.com/ruby/prism) and lints files
  in parallel across all cores.
- **Safe autocorrection.** `--fix` applies only edits that provably keep the
  program's behaviour. Everything else stays a report.
- **Not a RuboCop drop-in.** Cop names and `# rubocop:disable` comments are
  familiar, but there is no `.rubocop.yml` import and no style parameters.
  The defaults are the product.

## Status

Version 1.0.0, built from source; there is no published release yet.

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
(see [Development](#development)). Most cops are on by default; 34 Style
cops, 14 Layout cops (the line-break cops, `Layout/ClassStructure`,
`Layout/SingleLineBlockChain`, `Layout/MultilineAssignmentLayout`,
`Layout/EmptyLineAfterMultilineCondition` and
`Layout/HeredocArgumentClosingParenthesis`), five Lint cops
(`Lint/ConstantResolution`, `Lint/HeredocMethodCallPosition`,
`Lint/NumberConversion`, `Lint/ShadowingOuterLocalVariable`,
`Lint/UnusedPrivateMethod`), `Bundler/GemComment`, `Bundler/GemVersion`,
`Gemspec/DependencyVersion`, `Naming/InclusiveLanguage` and
`Style/FrozenStringLiteralComment` are opt-in. A few cops apply only to some
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
  only syntax is then a parse error. Set `ruby_version = "4.0"` in
  `rustocop.toml` for code that runs on Ruby 4.0. Older Ruby versions are not
  supported. See [Ruby version](#ruby-version).
- **File-local analysis.** Every file is linted independently and in
  parallel. Cops that need cross-file state in RuboCop (for example
  `Lint/DuplicateMethods` across files) work within one file here.
- **Standard cops only.** All nine RuboCop departments are covered, with
  Metrics thresholds fixed at RuboCop's defaults. There is no support for
  extension gems such as rubocop-rails or rubocop-rspec.
- **No unsafe autocorrect, formatter or server mode.**

## Installation

Building needs Rust 1.99 or newer and a C compiler for the vendored Prism
parser.

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
rustocop --fix app lib             # apply safe corrections in place
rustocop --fix --fix-passes 1 app  # apply a single correction pass
cat file.rb | rustocop --stdin file.rb
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

## Fixed defaults

| Setting | rustocop |
| --- | --- |
| String literals | double quotes |
| Hash syntax | `a: 1`; shorthand `{x:}` and `{x: x}` both accepted |
| Raising exceptions | `raise`, never `fail` |
| Line length | 120 columns (the only numeric setting, see [Configuration](#configuration)) |
| `# frozen_string_literal: true` | not required unless the cop is enabled |

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
```

Validation is strict: unknown keys, unknown cops or departments, invalid
globs and unsupported Ruby versions are errors before any linting. Department tables accept only `enabled`.
`max` is accepted only by `Layout/LineLength`.

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
`"3.4"`, which is the default, and `"4.0"`. The version selects three things:

- the grammar. With `"4.0"`, a line can start with `&&` or `||`, and
  `private def name = command argument` is valid. With `"3.4"`, both are
  syntax errors, as in Ruby 3.4.
- the cops that apply. `Style/ReverseFind` runs only with `"4.0"`, and
  `Lint/RedundantRequireStatement` then also reports `require 'pathname'`.
- the check of corrected code. `--fix` validates each correction with the
  same grammar.

The Ruby 4.0 grammar is compared with Ruby 4.0.9 in the tests. The tests that
run corrected code compare it with Ruby 3.4.1 only.

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

The tests run in parallel through
[cargo-nextest](https://nexte.st). A full run needs Docker with 8 GB of
memory and about 4 GB of disk. It takes about 8 minutes the first time and
about 5 minutes once the build output exists. The source is mounted
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
`RUSTOCOP_RUBY40` names its executable. On macOS some tests fail, because
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

An early measurement on a 2,600-file Ruby project: about 43 ms warm on Apple
Silicon with a release build. A proper benchmark suite is still to come.

## License

MIT. Not affiliated with RuboCop.
