# Usage

- [Commands](#commands)
- [Files](#files)
- [Exit codes and severities](#exit-codes-and-severities)
- [Output formats](#output-formats)
- [Continuous integration](#continuous-integration)
- [Editors](#editors)
- [Autocorrection](#autocorrection)

## Commands

```sh
rustocop                           # lint the current directory
rustocop app lib                   # lint specific paths
rustocop --only Lint               # one department or cop (comma-separated)
rustocop --only Style/IpAddresses  # force-enable an opt-in cop
rustocop --except Lint/Debugger
rustocop --format github           # GitHub Actions annotations
rustocop --format offenses         # the number of offenses per cop
rustocop --format json             # RuboCop's JSON, for editor plugins
rustocop -a app lib                # apply safe corrections in place
rustocop -A app lib                # apply unsafe corrections too
rustocop -a --fix-passes 1 app     # apply a single correction pass
cat file.rb | rustocop --stdin file.rb
rustocop --show-ruby-version       # the Ruby version of the run and its source
rustocop --write-baseline          # silence today's offenses in rustocop.toml
rustocop --import-rubocop-config   # write a rustocop.toml from .rubocop.yml
```

`rustocop --help` lists every option. `--only` and `--except` are described in
[cop selection](configuration.md#cop-selection); `--write-baseline` and
`--import-rubocop-config` in the
[migration guide](migrating-from-rubocop.md).

## Files

Directories are searched for `.rb` files, `Gemfile`, `*.gemfile`, `*.gemspec`,
`Gemfile.lock` and `gems.locked`. The search honours `.gitignore` and skips
hidden files. Other files, such as `Rakefile`, `*.rake` or `bin/` scripts, are
linted when named explicitly: `rustocop lib Rakefile`. `db/schema.rb`,
`vendor/**` and `node_modules/**` are excluded by default.

## Exit codes and severities

| Code | Meaning |
| ---: | --- |
| 0 | No offenses |
| 1 | Offenses remain |
| 2 | Errors: configuration, unreadable or unparseable files, internal cop errors |

Other files continue after a file error, and other cops continue after a cop
crash. Lint and Security offenses are reported as errors (`E`); the other
departments report conventions (`C`).

## Output formats

| `--format` | Output |
| --- | --- |
| `text` (default) | One line per offense, as RuboCop prints them |
| `github` | GitHub Actions workflow commands, shown as inline annotations |
| `offenses` | The number of offenses per cop, most first |
| `json` | The schema of RuboCop's `--format json` |

The JSON output differs from RuboCop's in these ways:

- An offense records where it starts and not where it ends. Each `location`
  is the one character at the start, so an editor marks that character and
  not the whole expression.
- `correctable` says that the cop has corrections, not that this offense has
  one. `corrected` is always `false`.
- `metadata` holds `rustocop_version` only, and the summary has an
  `error_count` when there are errors.
- A syntax error is an offense of `Lint/Syntax` with the severity `fatal`, as
  in RuboCop. Read errors and internal errors are strings in the `errors` of
  their file.

## Continuous integration

In GitHub Actions, `--format github` turns every offense into an inline
annotation:

```yaml
- run: rustocop --format github
```

## Editors

With `--format json`, an editor plugin written for RuboCop can run rustocop
when you name it as the executable. rustocop accepts the `--force-exclusion`
and `--server` options that those plugins pass, and ignores them: the
excludes of the config always apply, and there is no server.

```lua
-- Neovim, nvim-lint
require("lint").linters.rubocop.cmd = "rustocop"
```

```vim
" Vim, ALE
let g:ale_ruby_rubocop_executable = 'rustocop'
```

Fix-on-save integrations that run `rubocop -a --stdin` do not work yet:
corrections cannot be combined with `--stdin`.

## Autocorrection

`-a` (also `--fix` or `--autocorrect`) applies the corrections that
`rubocop -a` applies, the ones RuboCop marks safe. `-A` (`--autocorrect-all`)
also applies the ones RuboCop marks unsafe, as `rubocop -A` does, for the cops
whose unsafe corrections have been ported. Without one of these options, files
are never written.

```console
$ rustocop -a app
...
1 file inspected, 1 correction applied, 4 offenses remaining
```

As in RuboCop, a correction is applied even where the program could observe
it, for example when it moves a line before a `__LINE__` or changes the bytes
a program reads from its own `DATA` section.

**Passes.** Corrections are applied in passes. Each pass applies its edits to
the source, then lints the result again, so the next pass sees accurate
positions and any corrections the last one made possible. Overlapping edits
are resolved deterministically; the losing edit waits for the next pass. The
passes repeat in memory until one changes nothing, and the file is written
once. `--fix-passes N` stops after N passes; `--fix-passes 1` applies exactly
one.

**Guards.** Two guards stop a file whose corrections do not settle: a pass
that restores an earlier version of the file, and a limit of 200 passes.
Either one is reported as an error for that file, which keeps the last
validated version.

**Writing.** Corrected source is parsed again before an atomic replacement
that preserves permissions and symlinks. If the first pass fails validation,
the original is left unchanged; if a later pass does, the earlier passes are
kept and the failure is reported. Path exclusions and files with syntax
errors are never rewritten. An offense on a line where its cop is disabled
is not corrected, but, as in RuboCop, an enabled offense's correction
applies in full even where it reaches such a line.
