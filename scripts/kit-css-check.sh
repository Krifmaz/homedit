#!/usr/bin/env bash
# Checks that the kit's stylesheets in web/static/css are the pinned kit's: tokens.css (design/out),
# kit.css (crates/kit-leptos/style) and app.css (the template's shell). With --fix, copies them over.
set -euo pipefail
cd "$(dirname "$0")/../web"
manifest=$(cargo metadata --format-version 1 --locked | python3 -c '
import json, sys
m = json.load(sys.stdin)
print(next(p["manifest_path"] for p in m["packages"] if p["name"] == "kit-leptos"))')
kit=$(cd "$(dirname "$manifest")/../.." && pwd)
status=0
for pair in "design/out/tokens.css:tokens.css" "crates/kit-leptos/style/kit.css:kit.css" "template/web/static/css/app.css:app.css"; do
  src="$kit/${pair%%:*}"
  dst="static/css/${pair##*:}"
  if [[ "${1:-}" == "--fix" ]]; then
    cp "$src" "$dst"
  elif ! cmp -s "$src" "$dst"; then
    echo "web/$dst differs from the kit's ${pair%%:*} (run scripts/kit-css-check.sh --fix)" >&2
    status=1
  fi
done
exit $status
