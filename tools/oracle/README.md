# RuboCop spec oracle

The oracle runs RuboCop's own `expect_offense` / `expect_no_offenses` spec
examples against the `rustocop` binary and reports every miss and false
positive by line and column. Known deviations are listed in `baseline.txt`
with a reason each; the oracle fails on any deviation not listed there, and
on listed entries that no longer deviate.

## Running

Local only. Needs Ruby (2.7+), a RuboCop source checkout and RuboCop's
runtime gems (`gem install rubocop`, or `bundle install` in the checkout):
the specs are evaluated, not parsed, and they use RuboCop's own constants and
`config/default.yml`.

```sh
tools/oracle/oracle.rb Lint/Debugger            # one cop (or several)
tools/oracle/oracle.rb                          # every registered cop
tools/oracle/oracle.rb -v Style/Semicolon       # also known deviations, skips, errors
tools/oracle/oracle.rb --write-baseline Lint/Void   # accept the current deviations
cargo test --test integration rubocop_oracle::  # what `cargo test` runs
ruby tools/oracle/test/discovery_test.rb        # offline, no checkout/gems needed
ruby tools/oracle/test/oracle_test.rb           # shim regression tests (checkout/gems)
```

The checkout is `--rubocop PATH`, else `$RUBOCOP_SRC`, else `~/.cache/rubocop`.
The binary is `--binary PATH`, else a fresh `cargo build` in
`${CARGO_TARGET_DIR:-target}/debug`. The set of cops comes from the live Rust
registry (`cargo run --example list_cops`).

`cargo test` runs the oracle for every registered cop, plus the oracle's own
tests. It skips, passing, without Ruby, the checkout or its gems, and when the
checkout is not at the RuboCop revision `baseline.txt` records; check that
revision out to run it:

```sh
git -C "$RUBOCOP_SRC" fetch --depth 1 origin <revision> && git -C "$RUBOCOP_SRC" checkout <revision>
```

## Corrections

An example that calls `expect_correction` or `expect_no_corrections` is also
corrected: its source is copied, `rustocop --only COP -A` runs on the copy
(`--fix-passes 1` for `loop: false`), and the result is compared with the
source the spec expects. Each cop's line and the last line count the
corrections that match and differ; `-v` shows each difference. The count
does not make the run fail, and the baseline does not list it.

## Cases of our own

`cases/<department>/<cop>_spec.rb` holds examples beyond RuboCop's specs, in
the same format (`expect_offense`, `expect_no_offenses`, `expect_correction`),
such as the settings of `rubocop-rails-omakase` in `cases/layout/` and
`cases/style/`. The oracle runs them with the cop's own specs. RuboCop is their
oracle: as a case is evaluated, the pinned RuboCop cop runs on its source
under its configuration, and an offense, message or correction that RuboCop
does not give makes the example an evaluation error, which fails the run. Write
a case with what you expect; the error then names what RuboCop gives.

## Reference data

`style_reference.json` and `lint_reference.json` record, for the pinned
RuboCop revision, each cop's source and spec paths, upstream defaults,
Ruby version bounds and rustocop's fixed option overrides. Style discovery
uses these exact paths (including both `ConditionalAssignment` spec files)
rather than a prefix glob, which could load a neighboring cop. Other
departments use conventional snake-case paths. Missing source files or any
missing mapped spec are errors. `tools/fixed_policy.rb` turns the reference
data into the RuboCop configuration the oracle lints with.

## Which examples run

Every example whose configuration rustocop can represent runs. A setting
that a cop declares (`Cop::settings`, which `cargo run --example list_cops --
--settings` prints) is written to the `rustocop.toml` of the example, so its
examples run as soon as the cop declares it. The last line of a run counts
the skipped examples by reason; `config` is the number that falls as cops
gain settings. Examples that
need a setting rustocop fixes differently, a Ruby version outside the fixed
3.4 target, or RuboCop features rustocop does not model (project indexes,
mocks, Prism-excluded specs) are skipped and counted by reason. `-v` lists
them. Examples that raise are reported as evaluation errors, never silently
dropped.

Baseline refresh refuses missing specs and unevaluated examples; these cannot
be accepted as ordinary deviations. After `--write-baseline`, replace each new
entry's `TODO: triage` with the reason: a documented deviation, a bug to fix,
or why the example does not apply. The reasons of existing entries are kept.

## Attribution

The examples come from [RuboCop](https://github.com/rubocop/rubocop), read at
run time from a local checkout. Ported Rust tests and `tests/fixtures/` retain
attribution, and `baseline.txt` quotes example descriptions. The annotation
parsing in `rspec_shim.rb` (`format_offense`, `AnnotatedSource.parse`) and
the emulation of the `config` shared context are ported from RuboCop's
`lib/rubocop/rspec/`. RuboCop is distributed under the MIT License:

> Copyright (c) 2012-26 Bozhidar Batsov
>
> Permission is hereby granted, free of charge, to any person obtaining
> a copy of this software and associated documentation files (the
> "Software"), to deal in the Software without restriction, including
> without limitation the rights to use, copy, modify, merge, publish,
> distribute, sublicense, and/or sell copies of the Software, and to
> permit persons to whom the Software is furnished to do so, subject to
> the following conditions:
>
> The above copyright notice and this permission notice shall be
> included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
> EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
> MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
> NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
> LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
> OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
> WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
