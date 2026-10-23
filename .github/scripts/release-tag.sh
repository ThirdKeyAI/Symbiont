#!/usr/bin/env bash
# Release names must remain data in file commands, paths, and formula updates.
set -euo pipefail
number='(0|[1-9][0-9]*)'
identifier='[0-9A-Za-z-]+'
pattern="^v?${number}\.${number}\.${number}(-${identifier}(\.${identifier})*)?(\+${identifier}(\.${identifier})*)?$"
if [[ ! ${RELEASE_TAG:-} =~ $pattern ]]; then
  printf '%s\n' 'Invalid release tag: expected an optional v followed by a version.' >&2
  exit 1
fi
printf 'tag=%s\nversion=%s\n' "$RELEASE_TAG" "${RELEASE_TAG#v}" >> "$GITHUB_OUTPUT"
