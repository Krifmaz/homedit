#!/usr/bin/env bash
# Smoke test for a built homedit image.
#
#   scripts/smoke-test.sh <image>
#
# Checks that the image is under MAX_IMAGE_MB (default 40), that the
# container becomes healthy, that /healthz, /readyz and /metrics answer, and
# that idle memory after start-up stays under MAX_IDLE_MEM_MB (default 50).
set -euo pipefail

image="${1:?usage: smoke-test.sh <image>}"
max_image_mb="${MAX_IMAGE_MB:-40}"
max_mem_mb="${MAX_IDLE_MEM_MB:-50}"
name="homedit-smoke-$$"
port="${SMOKE_PORT:-38080}"

cleanup() {
  docker logs "$name" 2>&1 | tail -n 50 || true
  docker rm -f "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

size_bytes=$(docker image inspect --format '{{.Size}}' "$image")
size_mb=$(( (size_bytes + 1048575) / 1048576 ))
echo "image size: ${size_mb} MB (limit ${max_image_mb} MB)"
if (( size_mb > max_image_mb )); then
  echo "FAIL: image is larger than ${max_image_mb} MB" >&2
  exit 1
fi

docker run -d --name "$name" -p "127.0.0.1:${port}:3000" "$image" >/dev/null

echo "waiting for the container to report healthy..."
for _ in $(seq 1 60); do
  status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$name")
  case "$status" in
    healthy) break ;;
    exited|dead) echo "FAIL: container stopped" >&2; exit 1 ;;
  esac
  sleep 1
done
if [[ "$status" != healthy ]]; then
  echo "FAIL: container is $status after 60 s" >&2
  exit 1
fi

for path in /healthz /readyz /metrics; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}${path}")
  echo "GET ${path} -> ${code}"
  if [[ "$code" != 200 ]]; then
    echo "FAIL: ${path} answered ${code}" >&2
    exit 1
  fi
done
curl -fsS "http://127.0.0.1:${port}/metrics" | grep -q '^homedit_build_info'

# Let start-up work settle, then measure.
sleep 15
mem=$(docker stats --no-stream --format '{{.MemUsage}}' "$name" | awk '{print $1}')
mem_mb=$(awk -v m="$mem" 'BEGIN {
  v = m + 0
  if (m ~ /GiB$/) v *= 1024
  else if (m ~ /KiB$/) v /= 1024
  else if (m ~ /B$/ && m !~ /[KMG]iB$/) v /= 1048576
  printf "%d", v + 0.5
}')
echo "idle memory: ${mem} (~${mem_mb} MB, limit ${max_mem_mb} MB)"
if (( mem_mb > max_mem_mb )); then
  echo "FAIL: idle memory is above ${max_mem_mb} MB" >&2
  exit 1
fi
echo "smoke test passed"
