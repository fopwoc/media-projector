#!/usr/bin/env bash

set -euo pipefail

tag=${1:?usage: release-notes.sh TAG}

if [[ ! "$tag" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "tag is not a stable semantic version: $tag" >&2
    exit 1
fi

previous_tag=""
if git rev-parse --verify --quiet "${tag}^" >/dev/null; then
    previous_tag=$(
        git tag --merged "${tag}^" --sort=-version:refname |
            awk '/^[0-9]+\.[0-9]+\.[0-9]+$/ { print; exit }'
    )
fi

printf 'Changes:\n\n'
if [[ -n "$previous_tag" ]]; then
    git log --first-parent --format='- %s (%h)' "$previous_tag..$tag"
else
    git log --first-parent --format='- %s (%h)' "$tag"
fi

