#!/bin/sh
# Build and test on the minimum Rust version declared in Cargo.toml.
set -eu

cd "$(dirname "$0")/.."
msrv=$(awk -F '"' '/^rust-version[[:space:]]*=/ { print $2; exit }' Cargo.toml)
if [ -z "$msrv" ]; then
    echo "error: Cargo.toml does not declare rust-version" >&2
    exit 1
fi
# rustup toolchain names include the patch component.
case "$msrv" in
    *.*.*) toolchain=$msrv ;;
    *) toolchain=$msrv.0 ;;
esac

# Install explicitly with: rustup toolchain install <version> --profile minimal
# Do not silently install a toolchain or fall back to the default compiler.
cargo "+$toolchain" test --locked
