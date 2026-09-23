#!/usr/bin/env bash
# Build the SBF program, then run unit and LiteSVM integration tests.
#
# Usage: ./scripts/test.sh [cargo test args]
#
# Feature flags are forwarded to BOTH commands; every other argument goes to
# `cargo test` alone. Both halves matter:
#
#   - Features must reach the build. Otherwise `--features devnet` compiles a mainnet
#     program and runs the devnet-featured harness against it, which fails every priced
#     instruction with PriceAccountMismatch and does not say why.
#   - Nothing else may reach the build. `cargo build-sbf` takes
#     `[OPTIONS] [-- <cargo_args>]` and rejects `--lib`, `--test X` and bare test-name
#     filters outright, so forwarding "$@" wholesale breaks `./scripts/test.sh --lib`
#     (README) and every `--test X` invocation. A developer who hits that falls back to
#     plain `cargo test`, which reuses a stale `.so` — the exact footgun this script
#     exists to prevent.
set -euo pipefail
cd "$(dirname "$0")/.."

# Pick out feature flags for the build without consuming them from "$@".
build_args=()
prev=""
for arg in "$@"; do
    case "$arg" in
        --features=*|--no-default-features|--all-features) build_args+=("$arg") ;;
    esac
    # `--features X` spelled with a space carries its value in the next argument.
    if [ "$prev" = "--features" ]; then build_args+=(--features "$arg"); fi
    prev="$arg"
done

# `${x[@]+"${x[@]}"}` so an empty array is not an unbound-variable error under
# `set -u` on bash 3.2, which is what macOS ships.
cargo build-sbf --tools-version v1.52 ${build_args[@]+"${build_args[@]}"}
cargo test -p hodl_loans "$@"
