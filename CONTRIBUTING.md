# Contributing

Bug reports are most useful with a short Ruby snippet, the command you ran,
and what RuboCop 1.92 reports or corrects for the same code.

- [Building](#building)
- [Code layout](#code-layout)
- [Tests](#tests)
- [RuboCop spec oracle](#rubocop-spec-oracle)
- [Differential check](#differential-check)
- [Continuous integration](#continuous-integration)

## Building

Rust 1.99 or newer, a C compiler for the vendored Prism parser, and libclang.

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
tools/check-msrv.sh                       # builds and tests on Rust 1.99
cargo run -- path/to/file.rb
echo 'binding.pry' | cargo run -- --stdin x.rb
cargo run --example list_cops             # every registered cop name
cargo run --example list_cops -- --settings   # the settings cops declare
```

fmt, clippy and the tests must all be clean before a commit.

## Code layout

Every cop lives in its own file under `src/cops/<department>/`, implements the
`Cop` trait and is registered in `Registry::new()` in `src/cops/mod.rs`.
`src/cops/lint/debugger.rs` is the reference implementation. A cop that
builds facts once per file declares a `Fact` static in its own file (see
`Fact` in `src/source.rs`) instead of adding a field to `SourceFile`. Tests sit next to
the cop and in `tests/`. Where semantics match RuboCop, its spec examples are
ported; RuboCop is MIT licensed and the ported modules keep the attribution.

## Tests

**Layout.** Every file in `tests/` is a module of one test program,
`integration`. `tests/main.rs` lists the modules, so add a line there for a
new test file; a test fails when a file is missing from the list. Shared
helpers are in `tests/support/`. Run the tests of one file by its module name:

```sh
cargo test --test integration cli::
```

**In Docker.** The tests assume one environment: Ubuntu 24.04, its system Ruby
3.2 at `/usr/bin/ruby`, Ruby 3.4.1 and Ruby 4.0.9 built against the system
OpenSSL, and the RuboCop checkout at the pinned revision.
`tools/docker-test.sh` builds an image with exactly that and runs every check
of CI in it (`tools/check.sh`): fmt, clippy, rustdoc, the tests, the tests of
the Ruby tools in `tools/test`, and a check that each generated source is the
output of its generator. It gives the same result on every machine, and the
oracle always runs there. Pass a command to run something else:

```sh
tools/docker-test.sh                      # every check of CI
tools/docker-test.sh tools/check.sh test  # one step: lint, test, test-ruby-4.0 or tools
tools/docker-test.sh cargo test --lib     # any other command
```

The tests run in parallel through [cargo-nextest](https://nexte.st), in two
passes. The first pass runs every test, and the tests that start a Ruby
program use Ruby 3.4.1. The second pass sets `RUSTOCOP_TEST_RUBY=4.0` and runs
the integration tests again with Ruby 4.0.9. A test that is valid on Ruby 3.4
only starts with `if crate::support::runtime::skips("reason") { return; }`, so
the second pass skips it and the reason stays in the code.

A full run needs Docker with 8 GB of memory and about 4 GB of disk. It takes
about 15 minutes the first time and about 13 minutes once the build output
exists. The source is mounted read-only. Build output stays in the Docker
volume `rustocop-test-target` (about 1.4 GB); remove it with
`docker volume rm rustocop-test-target`.

**Natively.** Many tests run `ruby` from `PATH` and compare its output, so
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
integration tests use that executable in place of the Ruby 3.4 one. On macOS
some tests fail, because `/usr/bin/ruby` is Ruby 2.6 there and the temporary
directory sits behind a symlink; use Docker.

## RuboCop spec oracle

`tools/oracle` runs RuboCop's own spec examples for every registered cop
against the binary and reports misses and false positives by line and column.
Examples with `expect_correction` are also corrected with `rustocop -A`, and
the result is compared with the source the spec expects. `cargo test` runs the
oracle when Ruby, RuboCop's runtime gems and a checkout at the pinned revision
are available, and skips otherwise. Known deviations are listed with reasons
in `tools/oracle/baseline.txt`. Examples that RuboCop's specs lack go in
`tools/oracle/cases/<department>/<cop>_spec.rb`, in the same format; the
oracle checks each of them against RuboCop itself. See
[tools/oracle/README.md](tools/oracle/README.md).

## Differential check

`tools/differential.rb` compares rustocop with a local RuboCop executable over
a directory of real Ruby code and lists every position where they disagree.
It is an optional local check and never executes the corpus.

## Continuous integration

GitHub Actions runs `.github/workflows/ci.yml` on every push to master, on
pull requests, and weekly:

- **lint**, **test**, **test-ruby-4.0** and **tools** each run that step of
  `tools/check.sh`, the last three in the test image: the tests on x86_64
  and arm64 Linux, the architectures of the Linux binaries. Reproduce a step
  with `tools/docker-test.sh tools/check.sh <step>`.
- **build (macOS)** builds the code of the macOS binaries and lints a file
  with it; **build (stable Rust)** builds with the latest Rust, which users
  who build from source usually have.
- **cargo deny** checks the dependencies against security advisories, and
  their licenses and sources against `deny.toml`.
- **workflows and scripts** runs actionlint, zizmor and shellcheck.
- **CI** passes when all of them do.

Pushing a `v*` tag runs `.github/workflows/release.yml`: CI on the tagged
commit, then the release with binaries for Linux and macOS. Dependabot
proposes updates of the actions, which are pinned by commit, and of the
crates.
