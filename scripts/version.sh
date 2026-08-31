#!/usr/bin/env bash

set -euo pipefail

if [[ -n "${VERSION:-}" ]]; then
    if [[ ! "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] &&
        [[ ! "$VERSION" =~ ^[0-9]{8}-[0-9a-f]{8,}$ ]]; then
        echo "invalid build version: $VERSION" >&2
        exit 1
    fi
    printf '%s\n' "$VERSION"
    exit 0
fi

repository_root=$(git rev-parse --show-toplevel)
release_tag=""
if [[ -z "$(git -C "$repository_root" status --porcelain)" ]]; then
    release_tag=$(
        git -C "$repository_root" tag --points-at HEAD --sort=-version:refname |
            awk '/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/ { print; exit }'
    )
fi
if [[ -n "$release_tag" ]]; then
    printf '%s\n' "$release_tag"
    exit 0
fi

commit_date=$(git -C "$repository_root" show -s --format=%cs HEAD | tr -d -)
commit_hash=$(git -C "$repository_root" rev-parse --short=8 HEAD)
printf '%s-%s\n' "$commit_date" "$commit_hash"
