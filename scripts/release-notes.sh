#!/usr/bin/env bash
# Prints Markdown release notes for the commits since the last `vX.Y.Z` tag.
#
#   scripts/release-notes.sh <version> [rev]
#
# Commits are grouped by their Conventional Commits type when they have one
# (feat, fix, ...); everything else is listed under "Changes". `(#123)` in a
# subject becomes a link to that pull request.
set -euo pipefail

version="${1:?usage: release-notes.sh <version> [rev]}"
rev="${2:-HEAD}"
repo="${GITHUB_REPOSITORY:-Krifmaz/homedit}"
last_tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' "$rev" 2>/dev/null || true)
range="${last_tag:+${last_tag}..}${rev}"

declare -A groups=()
order=(Features Fixes Performance Documentation Changes)
while IFS= read -r subject; do
  [[ -z "$subject" || "$subject" == chore\(release\)* ]] && continue
  case "$subject" in
    feat*:*)              group=Features ;;
    fix*:*)               group=Fixes ;;
    perf*:*)              group=Performance ;;
    docs*:*)              group=Documentation ;;
    *)                    group=Changes ;;
  esac
  line=$(sed -E "s|\(#([0-9]+)\)|([#\1](https://github.com/${repo}/pull/\1))|g" <<<"$subject")
  groups[$group]+="- ${line}"$'\n'
done < <(git log --no-merges --format='%s' "$range")

echo "## homedit v${version}"
echo
for group in "${order[@]}"; do
  if [[ -n "${groups[$group]:-}" ]]; then
    echo "### ${group}"
    echo
    printf '%s\n' "${groups[$group]}"
  fi
done
cat <<NOTES
### Install

\`\`\`sh
docker pull ghcr.io/${repo,,}:${version}
\`\`\`

Static binaries for Linux amd64 and arm64 are attached below, with \`SHA256SUMS\`.
NOTES
if [[ -n "$last_tag" ]]; then
  echo
  echo "**Full diff:** https://github.com/${repo}/compare/${last_tag}...v${version}"
fi
