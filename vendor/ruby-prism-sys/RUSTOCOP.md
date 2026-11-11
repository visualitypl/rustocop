# Local Prism parser safety, Ruby 3.4 grammar and continuation-evidence patches

This is the crates.io `ruby-prism-sys` 1.9.0 crate, including its bundled Prism
C sources, under the upstream MIT license (`LICENSE.md`). The local Cargo patch
makes the safety changes apply to both our direct dependency and ruby-prism's
transitive dependency for builds rooted in this repository (including library
and release builds). No Ruby runtime or source generation is needed. Cargo
ignores dependency manifests' patches: downstream consumers must carry this
same root patch. Packaging this local fork for registry installation needs a
separate solution before publication; a source checkout/build is
supported.

Only `vendor/prism-1.9.0/src/prism.c` differs from the upstream crate:

- `parse_pattern_primitive` checks the existing `PRISM_DEPTH_MAXIMUM` and returns
  a MissingNode with `PM_ERR_NESTING_TOO_DEEP`, like `parse_expression`. Nested
  capture/array/hash patterns otherwise bypass the expression recursion guard.
- `parse_pattern` checks the same limit before allocating any nodes, returning
  a MissingNode with `PM_ERR_NESTING_TOO_DEEP`. Label and keyword-rest hash
  patterns recurse through `parse_pattern_hash`'s values directly into
  `parse_pattern`, bypassing `parse_pattern_primitive` entirely.
- `parse_required_destructured_parameter` receives and increments the same depth
  counter (including its caller in `parse_parameters`). At the existing limit
  it returns the partial MultiTargetNode with `PM_ERR_NESTING_TOO_DEEP`, rather
  than recursing further. The ordinary parser recovery owns and frees it.

The binding also adds an observation-only, thread-local continuation callback in
`prism.c`, scoped/cleared by `src/parser/continuations.rs`. Code-mode escaped
newlines report their byte offset, preceding token end, normal-newline ignore
state (including the pattern/keyword-argument exception), and resumed code
position after queued heredocs. The existing header-adjacent heredoc fast path
also reports its skipped code escape. These hooks do not change tokens, parser
state, ASTs, diagnostics, parser layouts, headers, or generated bindings. Literal
payloads/comments never become candidate escapes. This is narrow evidence for
Style/RedundantLineContinuation, not a token/lexer API or an alternate parse.
The callback ABI is declared manually in Rust; keep its parameter order/types
in sync. It is synchronous and cannot unwind through C. Tests cover LF/CRLF,
legacy multibyte trailing backslashes, queued bodies, thread isolation and
unwind cleanup. The lazy detector index is completed by the ordinary cop walk.

The exposed `when not ...` exception is superseded by a single
Ruby-3.4-only gate on the `PM_TOKEN_KEYWORD_NOT` prefix guard. Native Ruby 3.4.1's
**default Prism parser** allows English `not` in value/argument positions even
where it forbids direct command operands. Prism 1.9 instead ties bare `not`
acceptance to command acceptance; skip only that new guard in fixed-3.4 mode.
Do not change `accepts_command_call` at any call site. This restores arrays,
hash values, splats, later arguments, parameter defaults, ternaries, operator
operands, rescue classes and nested when operands without admitting ordinary
commands there. Other versions and Prism's latest/default API are unchanged.

`src/parser/english_not.rs` records 295 controlled valid/invalid witnesses
(56 contexts × five operand forms, plus 15 edge cases). The same table runs
unconditionally against production parsing and optionally against native 3.4:

```sh
RUSTOCOP_RUBY34=/path/to/ruby-3.4 cargo test parser::english_not --lib
RUSTOCOP_RUBY34=/path/to/ruby-3.4 cargo test --test cli ruby34_nested_english_not
```

The native witness explicitly uses `--parser=prism`, checks Ruby 3.4.x, and also
runs binding/effect observations. Ruby 3.4's alternate `--parser=parse.y` rejects
some of these sources (including `[not false]`); it is **not** the compatibility
oracle. Existing ordinary `if not false` was already accepted; tests retain it.
Tests also assert negation's receiver/locations, latest API isolation, comment/
newline handling, and invalid command/composition/pattern boundaries. Correction
test syntax checks now use `crate::parser::parse`, not latest-mode parsing.

The binding also adds a Ruby-3.4-only `PM_CONTEXT_BLOCK_PARAMETERS` case in
`parse_block_exit`. A block's own parameter defaults do not establish a target
for `next`, `break` or `redo` in native Ruby 3.4 Prism. Continue through that
context to the enclosing scope instead of accepting it. An enclosing loop,
block body, lambda body or `defined?` still permits the exit; a nested method
boundary still rejects it. The existing pending-exit lists handle trailing
`while`/`until` modifiers, including a modifier on the containing block call.
Nested default-owned loop/block/lambda bodies retain their own valid contexts.
No new validator, diagnostic, parse, node walk or parser state is introduced.
Latest/default and other grammar versions retain upstream behavior.

`src/parser/flow_defaults.rs` and `tests/block_default_flow.rs` share 241 authored
original-byte source-only witnesses in `tests/support/flow_defaults.rs`. They
cover exact messages/ranges, owned main-parse diagnostics and lazy metadata,
native `--parser=prism -c`, encoded/BOM/CRLF/heredoc inputs, enclosing and nested
scope boundaries, mandatory fatal delivery, every ordinary callback, selectors,
directives, strict config, stdin, observers and no-write/atomic-peer controls.
The three historical gap rows remain byte-for-byte historical fixture evidence;
their Rust consumer now requires native-invalid fatal rejection for them too.

The existing expression limit (10,000) is unchanged, so ordinary syntax errors
and the AST-depth diagnostic retain precedence/positions. No lexical nesting
heuristic or file-size limit is used. An isolated 64 MiB stack in `src/parser.rs`
provides headroom for C frames up to this parser limit on default Rust workers.
Rejected ASTs still require iterative teardown: parser recursion depth does not
bound AST paths produced by iterative parsing (e.g. long receiver chains).

Upgrade checklist:

1. Audit all three guarded recursion paths and the upstream cap; remove local
   changes if equivalent guards have landed. Include the label/keyword-rest hash
   value cycle (`parse_pattern` → `parse_pattern_hash` → `parse_pattern`), not
   just pattern primitives. Compare the vendored C diff with the upstream crate;
   it should contain only these three guards/destructured depth plumbing and
   the observation hooks, the Ruby-3.4 not-prefix guard gate and the
   Ruby-3.4 block-parameter context transparency in `parse_block_exit`.
   Recheck block-default flow boundaries against native Ruby 3.4 Prism on
   upgrades, including outer targets, nested method barriers and trailing loops.
   Compare English-not grammar with native Ruby 3.4's default Prism parser on
   upgrades; remove the gate once the fixed-version parser accepts the same
   sources upstream. Never enable ordinary commands to fix not-prefix parsing.
   Re-audit newline-ignore state, both heredoc
   resume paths, and the manually declared callback ABI against lexer changes.
   Keep both binding versions pinned together. The separate
   [ruby-prism binding patch](../ruby-prism/RUSTOCOP.md) supplies explicit Ruby 3.4
   options at our production entrypoint without changing C's latest/default mode;
   audit that narrow option/default/ownership plumbing on upgrades too.
2. Audit stack headroom and `src/parser/cleanup.rs` against the new C layouts.
3. Run the parser tests, default-worker parallel CLI regressions, correction
   no-write tests, the full suite/oracle, and MSRV verification. Test beyond the
   parser cap, not just beyond our 64-node analysis cap, in debug and release.

Retain the upstream files/license when refreshing this directory. The generated
crate manifest, build scripts and C/header files are otherwise unmodified.
