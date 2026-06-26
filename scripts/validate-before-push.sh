#!/bin/sh
set -eu

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

run cargo fmt --all -- --check
run cargo check --workspace --all-targets --all-features
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run cargo test --workspace --all-features
run cargo doc --workspace --all-features --no-deps

printf '\nLocal push validation passed.\n'
