#!/usr/bin/env bash
# Bind every build to an existing version tag on reviewed main history.
set -euo pipefail
source "$(dirname "$0")/release-tag.sh"
sha=$(git rev-parse --verify "refs/tags/${RELEASE_TAG}^{commit}")
git merge-base --is-ancestor "$sha" refs/remotes/origin/main || {
  printf '%s\n' 'Release tag is not on main history.' >&2
  exit 1
}
if [[ ${RELEASE_EVENT:-} == release && ${RELEASE_EVENT_SHA:-} != "$sha" ]]; then
  printf '%s\n' 'Release event and tag identify different commits.' >&2
  exit 1
fi
printf 'sha=%s\n' "$sha" >> "$GITHUB_OUTPUT"
