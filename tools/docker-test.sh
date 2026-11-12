#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
rust_version=$(awk -F '"' '/^rust-version[[:space:]]*=/ { print $2; exit }' Cargo.toml)
rubocop_revision=$(sed -n 's/^# RuboCop revision: //p' tools/oracle/baseline.txt)
if [ -z "$rust_version" ] || [ -z "$rubocop_revision" ]; then
    echo "error: could not read the Rust version or the RuboCop revision" >&2
    exit 1
fi

image=rustocop-test
docker build \
    --file Dockerfile.test \
    --build-arg RUST_VERSION="$rust_version" \
    --build-arg RUBOCOP_REVISION="$rubocop_revision" \
    --tag "$image" \
    .

exec docker run --rm \
    --volume "$PWD:/work:ro" \
    --volume rustocop-test-target:/target \
    --volume rustocop-test-registry:/usr/local/cargo/registry \
    "$image" "$@"
