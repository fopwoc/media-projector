#!/usr/bin/env bash

set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

output_dir=${1:-dist}
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)

container_engine=${CONTAINER_ENGINE:-docker}
version=$(./scripts/version.sh)
image_name="media-projector-release-$$"
container_id=""
temporary_dir=$(mktemp -d)

cleanup() {
    if [[ -n "$container_id" ]]; then
        "$container_engine" rm --force "$container_id" >/dev/null 2>&1 || true
    fi
    "$container_engine" image rm "$image_name" >/dev/null 2>&1 || true
    rm -rf "$temporary_dir"
}
trap cleanup EXIT

"$container_engine" build \
    --platform linux/amd64 \
    --build-arg "VERSION=$version" \
    --tag "$image_name" \
    .
reported_version=$("$container_engine" run --rm --platform linux/amd64 "$image_name" --version)
if [[ "$reported_version" != "media-projector $version" ]]; then
    echo "built binary reports unexpected version: $reported_version" >&2
    exit 1
fi
container_id=$("$container_engine" create --platform linux/amd64 "$image_name" --version)
"$container_engine" cp "$container_id:/media-projector" "$temporary_dir/media-projector"
install -m 0755 \
    "$temporary_dir/media-projector" \
    "$output_dir/media-projector-x86_64-linux"

if command -v sha256sum >/dev/null 2>&1; then
    (
        cd "$output_dir"
        sha256sum media-projector-x86_64-linux > SHA256SUMS
    )
else
    (
        cd "$output_dir"
        shasum -a 256 media-projector-x86_64-linux > SHA256SUMS
    )
fi
