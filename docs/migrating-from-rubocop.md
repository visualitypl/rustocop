# Migrating from RuboCop

- [Four steps](#four-steps)
- [Importing .rubocop.yml](#importing-rubocopyml)
- [Writing a baseline](#writing-a-baseline)
- [Differences from RuboCop](#differences-from-rubocop)

## Four steps

The output below is from RuboCop's own source, which has a `.rubocop.yml`.

1. Import the settings of `.rubocop.yml`:

   ```console
   $ rustocop --import-rubocop-config
   Wrote rustocop.toml from .rubocop_todo.yml and .rubocop.yml.
   Not carried over; rustocop has no setting for these:
     plugins
     AllCops: NewCops, SuggestExtensions, TargetRubyVersion, UseProjectIndex
     Layout/HashAlignment: EnforcedColonStyle, EnforcedHashRocketStyle
     ...
     not rustocop cops or departments: RSpec, RSpec/AnyInstance, Rake/MethodDefinitionInTask
   ```

2. See which cops report the most:

   ```console
   $ rustocop --format offenses
   378  Style/DisableCopsWithinSourceCodeDirective
   144  Layout/HashAlignment
   102  Lint/RedundantCopDisableDirective
   ...

   1766 files inspected, 1053 offenses detected
   ```

3. Apply the safe corrections:

   ```console
   $ rustocop -a
   ...
   1766 files inspected, 1362 corrections applied, 355 offenses remaining
   ```

4. Write a baseline for the rest:

   ```console
   $ rustocop --write-baseline
   Updated /home/me/rubocop/rustocop.toml: 355 offenses silenced, 4 cops switched off, 28 cops with skipped files.
   ```

A run with the baseline reports nothing, and each entry that you delete from
it brings its offenses back for you to fix.

Without a `.rubocop.yml`, start at step 2. On the same code without a config,
rustocop reports 56,152 offenses, mostly `Style/StringLiterals` because
RuboCop's source uses single quotes; `-a` corrects 54,364 of them.

## Importing .rubocop.yml

`rustocop --import-rubocop-config` reads the `.rubocop.yml` in the current
directory, and the local files that it names in `inherit_from`, such as
`.rubocop_todo.yml`. It writes a `rustocop.toml` next to it and lists the
settings that it does not carry over.

The import carries over:

- `AllCops: Exclude`, as the top-level `exclude`;
- `AllCops: TargetRubyVersion` when it is 3.4 or 4.0, as `ruby_version`;
- `Enabled: true` and `Enabled: false` of a cop or a department;
- `Exclude` of a cop;
- `Include` of a cop, when every entry is a relative glob;
- `Max` of `Layout/LineLength` and of a Metrics cop, when it is a whole
  number;
- `EnforcedStyle` of `Style/StringLiterals`,
  `Style/StringLiteralsInInterpolation`, `Style/QuotedSymbols`,
  `Style/HashSyntax`, `Layout/CaseIndentation` and
  `Layout/SpaceInsideArrayLiteralBrackets`;
- `EnforcedStyleAlignWith` of `Layout/EndAlignment`,
  `EnforcedStyleForEmptyBrackets` of `Layout/SpaceInsideArrayLiteralBrackets`
  and `EnforcedStyleForEmptyBraces` of `Layout/SpaceInsideBlockBraces`;
- `AllowMultipleReturnValues` of `Style/RedundantReturn` and
  `AllowAsExpressionSeparator` of `Style/Semicolon`;
- `AllowedPatterns` of `Layout/LineLength`. A pattern is a Ruby regexp, and
  the import keeps the ones that Rust's `regex` crate reads too. It lists the
  others, such as a pattern with a lookbehind.

RuboCop's default for string literals is single quotes and rustocop's is
double quotes. A config that names no quote style means RuboCop's default, so
the import writes `enforced_style = "single_quotes"` for the two string cops.
It does not when the config inherits from a gem or a URL, because it cannot
read what those say.

It lists everything else: other style parameters, cops that rustocop does not
have, `require`, `plugins`, `inherit_gem`, an `inherit_from` URL, and
`Exclude` entries that are absolute paths or regexps. A setting in
`.rubocop.yml` replaces the same setting of an inherited file, as in RuboCop;
`inherit_mode` with `merge: [Exclude]` at the top of a file joins the
`Exclude` lists.

The import never changes an existing config: when a `rustocop.toml` already
exists, it stops with an error.

## Writing a baseline

`rustocop --write-baseline` lints the given paths, or the current directory,
and writes a `rustocop.toml` with one entry per cop that has offenses. A cop
with offenses in at most 15 files gets an `exclude` list of those files, so it
still checks every other file and every new one. A cop with offenses in more
files is switched off with `enabled = false`.

When a `rustocop.toml` already exists, the run uses it and the baseline is
added to it: a cop that has a table there is switched off or gets more
`exclude` paths in that table, and any other cop gets a new table at the end
of the file. Everything else in the file stays as it is, comments included.

File errors, such as syntax errors, are listed and stay errors, because no
config silences them. The option cannot be combined with `-a`, `-A`,
`--stdin`, `--only`, `--except` or `--show-ruby-version`.

## Differences from RuboCop

With the same settings, every cop reports the offenses of its RuboCop 1.92.0
counterpart and makes the same corrections. The
[oracle](../tools/oracle/README.md) checks this against RuboCop's own spec
examples, and [`tools/oracle/baseline.txt`](../tools/oracle/baseline.txt)
lists the known deviations with reasons.

### Configuration

rustocop reads `rustocop.toml`, not `.rubocop.yml`. It supports a subset of
RuboCop's parameters, listed in [Configuration](configuration.md#settings);
every other parameter is fixed at RuboCop's default.

### Defaults

rustocop uses RuboCop's defaults, except:

| Setting | RuboCop | rustocop |
| --- | --- | --- |
| `Style/StringLiterals`, `Style/StringLiteralsInInterpolation` | single quotes | double quotes |
| Metrics cops | on | opt-in; a `max` switches a cop on |
| `Style/Documentation` | on | opt-in |
| `Style/FrozenStringLiteralComment` | on | opt-in |

### Opt-in cops

68 cops are off unless enabled: the four above, and the ones RuboCop also
ships disabled.

<details>
<summary>All opt-in cops</summary>

| Department | Cops |
| --- | --- |
| Bundler | `GemComment`, `GemVersion` |
| Gemspec | `DependencyVersion` |
| Layout | `ClassStructure`, `EmptyLineAfterMultilineCondition`, `FirstArrayElementLineBreak`, `FirstHashElementLineBreak`, `FirstMethodArgumentLineBreak`, `FirstMethodParameterLineBreak`, `HeredocArgumentClosingParenthesis`, `MultilineArrayLineBreaks`, `MultilineAssignmentLayout`, `MultilineHashKeyLineBreaks`, `MultilineMethodArgumentLineBreaks`, `MultilineMethodParameterLineBreaks`, `RedundantLineBreak`, `SingleLineBlockChain` |
| Lint | `ConstantResolution`, `HeredocMethodCallPosition`, `NumberConversion`, `ShadowingOuterLocalVariable`, `UnusedPrivateMethod` |
| Metrics | `AbcSize`, `BlockLength`, `BlockNesting`, `ClassLength`, `CollectionLiteralLength`, `CyclomaticComplexity`, `MethodLength`, `ModuleLength`, `ParameterLists`, `PerceivedComplexity` |
| Naming | `InclusiveLanguage` |
| Style | `ArrayCoercion`, `ArrayFirstLast`, `AsciiComments`, `AutoResourceCleanup`, `ClassMethodsDefinitions`, `CollectionMethods`, `ConstantVisibility`, `Copyright`, `DateTime`, `DisableCopsWithinSourceCodeDirective`, `Documentation`, `DocumentationMethod`, `FrozenStringLiteralComment`, `HashLookupMethod`, `ImplicitRuntimeError`, `InlineComment`, `InvertibleUnlessCondition`, `IpAddresses`, `MethodCallWithArgsParentheses`, `MethodCalledOnDoEndBlock`, `MissingElse`, `MultilineMethodSignature`, `OptionHash`, `RedundantStructKeywordInit`, `RequireOrder`, `ReturnNil`, `Send`, `SingleLineBlockParams`, `StaticClass`, `StringHashKeys`, `StringMethods`, `TopLevelMethodDefinition`, `TrailingCommaInBlockArgs`, `UnlessLogicalOperators`, `YodaExpression` |

</details>

See [cop selection](configuration.md#cop-selection) for how to switch them
on.

### Ruby versions

Source is parsed with Ruby 3.4 grammar by default, or with Ruby 4.0 grammar
when the project declares Ruby 4.0. Older target versions are checked as Ruby
3.4, so cops that apply only to older Rubies report nothing. See
[Ruby version](configuration.md#ruby-version).

### Scope

- **File-local analysis.** Every file is linted on its own and in parallel.
  Checks that look across files in RuboCop work within one file.
- **Standard cops only.** Extension gems such as rubocop-rails,
  rubocop-rspec and rubocop-performance are not supported.
- **No server mode or result cache.** Every run is a full lint.
- **JSON output** follows RuboCop's schema with a few
  [differences](usage.md#output-formats).
- **Unsafe corrections** (`-A`) are being ported cop by cop; a cop without
  one reports its offense and leaves the code as it is.
- **Corrections that break the file** are not written. Where RuboCop's
  correction produces invalid Ruby, rustocop applies the other cops'
  corrections and reports the cop whose edits it left out (see
  [Autocorrection](usage.md#autocorrection)).
