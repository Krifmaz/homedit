#!/usr/bin/env bash
# Prints the next release version, computed from the commits since the last
# `vX.Y.Z` tag, or nothing when there is nothing to release.
#
#   scripts/next-version.sh [rev]      (rev defaults to HEAD)
#
# Rules (Conventional Commits, with a patch release as the default so that
# every merge to main ships):
#   * no tag yet                          -> the version in Cargo.toml as is
#   * `type!:` or `BREAKING CHANGE`       -> major (minor while on 0.x)
#   * `feat:` / `feat(scope):`            -> minor
#   * anything else                       -> patch
#   * only `chore(release):` commits      -> nothing (no release)
set -euo pipefail

rev="${1:-HEAD}"
cargo_version=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -n1)
if [[ -z "$cargo_version" ]]; then
  echo "cannot read [workspace.package] version from Cargo.toml" >&2
  exit 1
fi

last_tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' "$rev" 2>/dev/null || true)
if [[ -z "$last_tag" ]]; then
  echo "$cargo_version"
  exit 0
fi

# Subjects and bodies of the commits since the tag, release commits excluded.
log=$(git log --format='%s%n%b%n--END--' "${last_tag}..${rev}" | awk '
  /^--END--$/ { if (keep) printf "%s", buf; buf = ""; keep = 0; first = 1; next }
  { if (first) { keep = ($0 !~ /^chore\(release\)/); first = 0 } buf = buf $0 "\n" }
  BEGIN { first = 1 }
')
if [[ -z "${log//[$'\n' ]/}" ]]; then
  exit 0
fi

IFS=. read -r major minor patch <<<"${last_tag#v}"
if grep -Eq '^[a-z]+(\([^)]*\))?!:|^BREAKING[ -]CHANGE' <<<"$log"; then
  if (( major == 0 )); then bump=minor; else bump=major; fi
elif grep -Eq '^feat(\([^)]*\))?:' <<<"$log"; then
  bump=minor
else
  bump=patch
fi

case "$bump" in
  major) echo "$((major + 1)).0.0" ;;
  minor) echo "${major}.$((minor + 1)).0" ;;
  patch) echo "${major}.${minor}.$((patch + 1))" ;;
esac
