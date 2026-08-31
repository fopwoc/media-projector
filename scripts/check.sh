#!/usr/bin/env bash

set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --release

version=$(./scripts/version.sh)
reported_version=$(./target/release/media-projector --version)
if [[ "$reported_version" != "media-projector $version" ]]; then
    echo "built binary reports unexpected version: $reported_version" >&2
    exit 1
fi
