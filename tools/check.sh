#!/bin/sh
# The checks that CI runs, by step. CI runs each step as its own job; with no
# arguments, this runs every step in order, as `tools/docker-test.sh` does.
#
#   lint           fmt, clippy and rustdoc, with warnings denied
#   test           every test, with Ruby 3.4.1
#   test-ruby-4.0  the integration tests again, with Ruby 4.0.9
#   tools          the tests of the Ruby tools, and every generated source
#                  against the output of its generator
#
# All but `lint` need the environment of the test image (Dockerfile.test).
set -eu

cd "$(dirname "$0")/.."

lint() {
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings
    RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --document-private-items --locked
}

test_ruby_3_4() {
    cargo build --locked
    cargo nextest run --locked --no-fail-fast
    cargo test --doc --locked
}

test_ruby_4_0() {
    RUSTOCOP_TEST_RUBY=4.0 cargo nextest run --locked --no-fail-fast --test integration
}

tools() {
    # The tools build the list_cops example offline, which needs the
    # dev-dependencies, and escape_encoding_matrix_test.rb lints with the
    # debug binary.
    cargo fetch --locked
    cargo build --locked
    tools_failed=0
    for test in tools/test/*_test.rb; do
        echo "ruby $test"
        ruby "$test" || tools_failed=1
    done
    generated || tools_failed=1
    return "$tools_failed"
}

# Regenerates each generated source, formats it as `cargo fmt` does, and
# compares it with the committed file.
generated() {
    if ! ruby -e 'exit RbConfig::CONFIG["UNICODE_VERSION"] == "15.0.0"'; then
        echo "error: the generators need a Ruby with Unicode 15.0, such as the test image's system Ruby" >&2
        return 1
    fi
    edition=$(awk -F '"' '/^edition[[:space:]]*=/ { print $2; exit }' Cargo.toml)
    scratch=$(mktemp -d)
    generated_failed=0
    # Each line: the file in src/source, the generator in tools and its arguments.
    while read -r name generator arguments; do
        file=src/source/$name.rs
        command="ruby tools/$generator${arguments:+ $arguments}"
        echo "$command"
        # shellcheck disable=SC2086 # $arguments is a list of words
        if ruby "tools/$generator" $arguments <"/dev/null" >"$scratch/$name.rs" \
            && rustfmt --edition "$edition" "$scratch/$name.rs" \
            && cmp -s "$file" "$scratch/$name.rs"; then
            continue
        fi
        echo "error: $file is not the formatted output of $command; regenerate it" >&2
        diff -u "$file" "$scratch/$name.rs" | head -n 40 >&2 || true
        generated_failed=1
    done <<EOF
constant_lowercase generate_constant_lowercase.rb
constant_uppercase_change generate_constant_lowercase.rb --uppercase-change
escape_alnum generate_escape_alnum.rb
heredoc_uppercase generate_heredoc_uppercase.rb
naming_uppercase generate_constant_lowercase.rb --uppercase
source_inspection generate_source_inspection.rb
word_array_classes generate_word_array_classes.rb
EOF
    rm -rf "$scratch"
    # Not checked: src/cops/lint/shadowed_exception_constants.tsv, whose
    # generator gives other ancestor orders in the test image.
    return "$generated_failed"
}

if [ $# -eq 0 ]; then
    set -- lint test test-ruby-4.0 tools
fi
for step in "$@"; do
    case $step in
        lint | test | test-ruby-4.0 | tools) ;;
        *)
            echo "usage: $0 [lint | test | test-ruby-4.0 | tools]..." >&2
            exit 2
            ;;
    esac
done
for step in "$@"; do
    echo "==> $step"
    case $step in
        lint) lint ;;
        test) test_ruby_3_4 ;;
        test-ruby-4.0) test_ruby_4_0 ;;
        tools) tools ;;
    esac
done
