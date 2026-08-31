#!/usr/bin/env bash

set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

image_name=${1:-media-projector}
version=$(./scripts/version.sh)
container_engine=${CONTAINER_ENGINE:-docker}
"$container_engine" build \
    --build-arg "VERSION=$version" \
    --tag "$image_name" \
    .
