# rustocop

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![RuboCop 1.92](https://img.shields.io/badge/RuboCop-1.92-red.svg)](docs/migrating-from-rubocop.md)
[![Ruby 3.4 | 4.0](https://img.shields.io/badge/Ruby-3.4%20%7C%204.0-CC342D.svg)](docs/configuration.md#ruby-version)

**A fast Ruby linter and autocorrector written in Rust, compatible with RuboCop.**

rustocop runs all 613 standard cops of RuboCop 1.92, with RuboCop's cop names,
disable comments and corrections. It is a single binary that needs no Ruby
installation, and it lints RuboCop's own 1,793 source files in under 0.4
seconds.

```console
$ rustocop app
app/order.rb:3:5: W: Remove debugger entry point `binding.pry`. (Lint/Debugger)
app/order.rb:4:15: C: Pass `&:price` as an argument to `map` instead of a block. (Style/SymbolProc)
app/order.rb:8:5: C: Prefer double-quoted strings unless you need single quotes to avoid extra backslashes for escaping. (Style/StringLiterals)
app/order.rb:8:17: C: Favor `format` over `String#%`. (Style/FormatString)

1 file inspected, 4 offenses detected
```

## Highlights

- **RuboCop's cops.** All nine standard departments, 613 cops, tested
  against RuboCop's own spec examples.
- **RuboCop's corrections.** `-a` applies what `rubocop -a` applies. `-A`
  adds RuboCop's unsafe corrections, which are being ported cop by cop.
- **Fast.** Parses with [Prism](https://github.com/ruby/prism) and lints files
  in parallel on every core.
- **No Ruby required.** One executable, without Ruby, Bundler or gems.
- **Fits existing projects.** It honours `# rubocop:disable` comments, imports
  `.rubocop.yml`, writes a baseline for legacy code, and prints RuboCop's JSON
  for editor plugins and annotations for GitHub Actions.

## Installation

Download a binary for Linux (x86_64 or aarch64, glibc 2.35 or newer) or macOS
(Intel or Apple silicon) from the
[releases page](https://github.com/visualitypl/rustocop/releases), and put
`rustocop` in a directory on your `PATH`.

To build from source you need Rust 1.99 or newer, a C compiler and libclang:

```sh
git clone https://github.com/visualitypl/rustocop.git
cd rustocop
cargo install --locked --path .
```

## Usage

```sh
rustocop                          # lint the current directory
rustocop app lib                  # lint some paths
rustocop -a                       # apply safe corrections, like rubocop -a
rustocop -A                       # apply unsafe corrections too, like rubocop -A
rustocop --only Lint,Style/Send   # run some departments or cops
rustocop --format github          # GitHub Actions annotations
```

The exit code is `0` when the code is clean, `1` when offenses remain and `2`
on errors. See [Usage](docs/usage.md) for every option, file discovery, output
formats, editor setup and how corrections are applied.

## Migrating from RuboCop

```console
$ rustocop --import-rubocop-config   # rustocop.toml from your .rubocop.yml
$ rustocop --format offenses         # which cops report the most
$ rustocop -a                        # apply the safe corrections
$ rustocop --write-baseline          # silence the rest, then fix it over time
```

On RuboCop's own source, the imported config brings 1,053 offenses, `-a`
corrects all but 355, and the baseline silences those. The
[migration guide](docs/migrating-from-rubocop.md) walks through each step and
lists where rustocop differs from RuboCop.

## Configuration

rustocop needs no configuration. An optional `rustocop.toml`, found by
searching upward from the current directory to the git root, excludes paths,
switches cops on or off and holds the settings rustocop supports:

```toml
exclude = ["bin/**", "spec/fixtures/**"]

[cops."Layout/LineLength"]
max = 100

[cops."Style/StringLiterals"]
enforced_style = "single_quotes"
```

Inline directives work as in RuboCop:

```ruby
binding.pry # rubocop:disable Lint/Debugger
```

See [Configuration](docs/configuration.md) for every key, the Ruby version and
all directives.

## RuboCop compatibility

With the same settings, rustocop reports the offenses RuboCop 1.92.0 reports
and makes the same corrections. An oracle runs RuboCop's spec examples for every cop against
rustocop, and the known deviations are listed in
[`tools/oracle/baseline.txt`](tools/oracle/baseline.txt). The differences:

- **Configuration** lives in `rustocop.toml`, which supports a growing subset
  of RuboCop's parameters. `--import-rubocop-config` carries over what has a
  counterpart.
- **A few defaults differ**: double quotes, and the Metrics cops,
  `Style/Documentation` and `Style/FrozenStringLiteralComment` are opt-in.
- **Ruby 3.4 and 4.0** syntax. Older target versions are checked as 3.4.
- **Each file is linted on its own**, there is no result cache or server mode,
  and extension gems such as rubocop-rails and rubocop-rspec are not supported.

The [migration guide](docs/migrating-from-rubocop.md#differences-from-rubocop)
has the details.

## Performance

Linting RuboCop's own source (1,793 files, about 390,000 lines) on an Apple
M5 Pro:

| Command                           | Mean ± σ        |
| --------------------------------- | --------------- |
| `rustocop .`                      | 371 ms ± 43 ms  |
| `RAYON_NUM_THREADS=1 rustocop .`  | 2.39 s ± 0.02 s |
| `rubocop --parallel`, empty cache | 6.57 s ± 0.16 s |
| `rubocop --parallel`, warm cache  | 775 ms ± 10 ms  |

rustocop keeps no cache, so every run is a full lint. See
[Performance](docs/performance.md) for the method.

## Documentation

- [Usage](docs/usage.md): options, file discovery, exit codes, output formats,
  editors, CI and autocorrection
- [Configuration](docs/configuration.md): `rustocop.toml`, cop selection, Ruby
  version and inline directives
- [Migrating from RuboCop](docs/migrating-from-rubocop.md): import, baseline
  and the differences from RuboCop
- [Performance](docs/performance.md): benchmark and method
- [Contributing](CONTRIBUTING.md): building, testing and the RuboCop spec
  oracle

## Contributing

Bug reports with a snippet where rustocop and RuboCop disagree are the most
useful. See [CONTRIBUTING.md](CONTRIBUTING.md) to build and test the project.

## License

[MIT](LICENSE). rustocop is not affiliated with RuboCop. Tests ported from
RuboCop's specs (MIT, © Bozhidar Batsov) keep their attribution.
