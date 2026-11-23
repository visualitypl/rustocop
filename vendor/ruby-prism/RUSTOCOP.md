# Local binding patches

This is the crates.io `ruby-prism` **1.9.0** crate, under its upstream MIT license
(`LICENSE.md`). Only `src/lib.rs` differs from that release; the manifest,
build script, generated-node input and other upstream files are unchanged.

## Fixed-grammar binding entrypoint

Upstream exposes only `parse(source)`, which passes null options and selects
Prism's evolving latest grammar (4.1 in the pinned C source). The local patch
adds **`parse_with_version(source, version)`** and **`parse_ruby_3_4(source)`**,
which is the same call with the CRuby 3.4 grammar. Both share the existing
allocation/initialization body through a private helper. The original `parse`
keeps its null-options behavior. There is no general options builder or public
rustocop target setting.

The entrypoint passes a zero-initialized `pm_options_t`, with only:

- `version` set to the requested grammar (`PM_OPTIONS_VERSION_CRUBY_3_4` for
  `parse_ruby_3_4`);
- `line = 1`, matching the null-options default (a zero options line would
  otherwise change source-line nodes and diagnostics);
- `main_script = true` **only** when Prism recognizes a Ruby shebang on the first
  line. This retains the null-options CRLF shebang warning without introducing
  implicit `-x` scanning for non-Ruby shebangs. No shebang callback is installed,
  so shebang command-line flags remain ignored, as with null options. The small
  predicate deliberately mirrors Prism 1.9's BOM handling, including its search
  starting at the original source with a length that excludes the BOM.

Zero preserves the normal unset frozen/chilled-string mode, UTF-8 with magic
comments enabled, empty filepath, no eval scopes, no command-line flags, and
ordinary non-partial script behavior. The options own no allocations and are
borrowed only during parser initialization; no pointer into that temporary
struct survives. `ParseResult`'s source lifetime, !Send contract and destructor
are unchanged. No parser layout transmutation, extra parse or C policy change
is needed.

`src/parser.rs` uses this entrypoint **inside** the existing same-thread stack
segment and scoped continuation observer. All production lint, metadata fallback
and correction-validation parses therefore use the Ruby version of the run
(3.4 by default), with the same preflight
and stack-bounded cleanup for deep/recovered trees. Existing unit fixtures that
call upstream `parse` directly still exercise upstream defaults.

## Lexer callback

**`parse_with_version_lexing(source, version, lex_callback)`** is
`parse_with_version` with Prism's own `pm_parser_t::lex_callback` installed
between `pm_parser_init` and `pm_parse`, and removed before it returns: the
hook `Prism.lex` uses. rustocop's main parse records the token stream with it
for `Layout/ExtraSpacing` and its peers, instead of lexing the file a second
time. No other option or parser state differs.

## Printable diagnostic messages

Upstream `Diagnostic::message()` assumes all C diagnostic messages are UTF-8
and panics on invalid sequences. Prism can interpolate raw source-encoded bytes
into both warnings (such as unused local names) and syntax errors (such as
missing pinned locals). Valid legacy-encoded Ruby can therefore trigger that
panic before any cop runs.

The local binding returns **`Cow<'_, str>`** instead of `&str`, using
`CStr::to_string_lossy()`: valid UTF-8 is borrowed without allocation, while
invalid sequences become U+FFFD in owned display text. This is deliberately not
charset transcoding; it preserves existing UTF-8 messages and makes arbitrary
encoded diagnostics printable without adding an encoding dependency. Callers
retaining messages use `into_owned()`.

Only message display changes. The parser still receives the original bytes;
AST names, source bytes, diagnostic locations and correction ranges are never
lossily decoded or re-encoded. The shared accessor covers errors and warnings
for both parse entrypoints, main-parse metadata recording and lazy fallbacks.
Regressions in `src/parser.rs` and `tests/encoding_diagnostics.rs` cover
ISO-8859-1, Windows-1252 and Shift_JIS, UTF-8/ASCII borrowing, byte ranges,
plain/stdin lint, no-write fixes, neighboring safe fixes and printable syntax
errors in both output formats.

## Upgrade and distribution checklist

1. Keep this binding and `ruby-prism-sys` pinned together. Prefer a suitable
   upstream options API if one becomes available; otherwise reapply only this
   small `src/lib.rs` diff. Never globally redefine Prism's latest version.
   Also retain safe diagnostic display (or adopt an upstream equivalent); audit
   accessor return types and owned-message call sites when replacing this patch.
2. Audit `pm_options_t` zero initialization and `pm_parser_init`'s null/non-null
   defaults, especially line, frozen mode, encoding, scope and ownership changes.
   Recheck the explicit version constant rather than its numeric value. Re-audit
   the Ruby-shebang predicate against `pm_parser_init` (including BOM handling),
   warning parity and the absence of implicit `-x` source skipping.
3. Follow [the sys patch checklist](../ruby-prism-sys/RUSTOCOP.md), including
   recursion guards, TLS callback ABI, stack headroom and rejected-tree teardown.
4. Run parser/continuation, CLI and correction-validation regressions, the full
   Rust suite, full pinned oracle and Rust 1.88 MSRV checks. Tests assert actual
   parser-version state, Ruby-4 rejection positions and Ruby-3.4 AST/flag/binding
   behavior, not merely cop applicability metadata.

The root Cargo patch applies to local library and binary builds. Like the sys
patch, it is **not inherited by downstream Cargo consumers**; they must carry
both root patches. Registry packaging of local parser forks is not solved;
no registry-install support or publication is claimed here.
