# rustocop

A fast, opinionated Ruby linter written in Rust.

rustocop ships 491 cops that behave like their RuboCop counterparts, but it
makes the style decisions for you, needs no Ruby installation to run, and
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

Version 0.5.0, built from source; there is no published release yet.

| Department | Cops |
| --- | ---: |
| Style | 299 |
| Lint | 145 |
| Layout | 21 |
| Naming | 19 |
| Security | 7 |

Each cop matches the behaviour of its RuboCop 1.92.0 counterpart at
rustocop's fixed settings and is verified against RuboCop's own spec examples
(see [Development](#development)). Most cops are on by default; 34 Style
cops, five Lint cops (`Lint/ConstantResolution`,
`Lint/HeredocMethodCallPosition`, `Lint/NumberConversion`,
`Lint/ShadowingOuterLocalVariable`, `Lint/UnusedPrivateMethod`),
`Naming/InclusiveLanguage` and `Style/FrozenStringLiteralComment` are
opt-in. A few cops apply only to Ruby versions other than the fixed 3.4 target
(`Security/YAMLLoad`, `Style/ReverseFind`, `Lint/ItWithoutArgumentsInBlock`,
`Lint/UselessElseWithoutRescue`, `Lint/NonDeterministicRequireOrder`); they
stay registered but report nothing.

## How it differs from RuboCop

- **One style.** Where RuboCop offers a choice (`EnforcedStyle`, thresholds,
  allow-lists), rustocop picks one and does not expose the setting. See
  [Fixed defaults](#fixed-defaults).
- **Ruby 3.4 only.** Source is parsed with Ruby 3.4 grammar, including
  implicit `it`, argument forwarding and encoding comments. Ruby 4 only syntax
  is a parse error. There is no target version setting.
- **File-local analysis.** Every file is linted independently and in
  parallel. Cops that need cross-file state in RuboCop (for example
  `Lint/DuplicateMethods` across files) work within one file here.
- **Five departments.** There is no Metrics, Bundler or Gemspec department and
  no support for extension gems such as rubocop-rails or rubocop-rspec.
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
cat file.rb | rustocop --stdin file.rb
```

Directories are searched for `.rb` files, honouring `.gitignore`. Other files
(`Rakefile`, `*.rake`, `bin/` scripts) are linted when named explicitly, for
example `rustocop lib Rakefile`. `db/schema.rb`, `vendor/**` and
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

Corrections are applied in one pass over the original source, then the file
is re-linted so remaining offenses have accurate positions. Overlapping edits
are resolved deterministically; the losing edit is reported again for the
next run. Corrected source is validated before an atomic replacement that
preserves permissions and symlinks, and a failed validation leaves the
original unchanged. Disabled lines, path exclusions and files with syntax
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

Validation is strict: unknown keys, unknown cops or departments and invalid
globs are errors before any linting. Department tables accept only `enabled`.
`max` is accepted only by `Layout/LineLength`.

Selection precedence, highest first: `--except`, `--only` (which also
force-enables opt-in cops), per-cop `enabled`, department `enabled`, the
cop's default. `--only Style` enables the opt-in Style cops too, including
ones that hold competing opinions. Unknown selectors are errors with a
did-you-mean suggestion where possible.

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
