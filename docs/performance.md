# Performance

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

## Method

Measured with hyperfine 1.18 (50 runs for rustocop, 5 and 10 for RuboCop,
files already in the page cache) on an Apple M5 Pro with 15 cores, under Linux
aarch64 in the `Dockerfile.test` image: a `--release` build of rustocop 1.0.0
with Rust 1.99.1, and RuboCop 1.92.0 on Ruby 3.2.3 with `NewCops: enable` and
no extensions.

The RuboCop rows are for scale, not a like-for-like race: the defaults differ,
and rustocop reports 56,152 offenses on this code where RuboCop reports 5,970.

To reproduce the rustocop rows, build with `cargo build --release` and run
`hyperfine --runs 50 'rustocop .'` in a checkout of RuboCop at the
revision above.
