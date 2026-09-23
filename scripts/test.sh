#!/usr/bin/env bash
# Build the SBF program, then run unit and LiteSVM integration tests.
# Feature flags MUST reach both: a devnet-featured harness against a default-featured
# .so fails every priced instruction with PriceAccountMismatch.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build-sbf --tools-version v1.52 "$@"
cargo test -p hodl_loans "$@"
