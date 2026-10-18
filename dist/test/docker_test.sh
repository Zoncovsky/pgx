#!/bin/sh
# Build the PGX image, run it, connect over TCP, check persistence across a
# container restart. Requires a running Docker daemon; otherwise a COVERAGE GAP.
#
#   dist/test/docker_test.sh
set -eu
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
command -v docker >/dev/null 2>&1 || { echo "COVERAGE GAP: Docker — docker not installed; the image is UNVERIFIED on this host"; exit 2; }
docker info >/dev/null 2>&1 || { echo "COVERAGE GAP: Docker — daemon not reachable; the image is UNVERIFIED on this host"; exit 2; }
command -v psql >/dev/null 2>&1 || { echo "COVERAGE GAP: Docker test needs a psql client"; exit 2; }
fail() { echo "FAIL: $*" >&2; exit 1; }
TAG="pgx-dockertest:$$"
VOL="pgxdockertest$$"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
NAME="pgxdt$$"
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; docker volume rm "$VOL" >/dev/null 2>&1 || true; docker rmi "$TAG" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM

echo "building image ..."
docker build -q -t "$TAG" "$ROOT" >/dev/null || fail "docker build"
echo "starting container ..."
docker run -d --name "$NAME" -e PGX_PASSWORD=dockerpass -p "$PORT:5432" -v "$VOL:/var/lib/pgx" "$TAG" >/dev/null || fail "docker run"
URL="postgres://postgres:dockerpass@127.0.0.1:$PORT/postgres"
ok=""
for _ in $(seq 1 60); do
  if psql "$URL" -Atqc "SELECT 1" >/dev/null 2>&1; then ok=1; break; fi
  sleep 1
done
[ -n "$ok" ] || { docker logs "$NAME" | tail -20; fail "engine did not accept TCP connections"; }
psql "$URL" -Atqc "CREATE TABLE persist (v text); INSERT INTO persist VALUES ('survives')" >/dev/null || fail "write failed"
docker restart "$NAME" >/dev/null || fail "restart"
for _ in $(seq 1 60); do psql "$URL" -Atqc "SELECT 1" >/dev/null 2>&1 && break; sleep 1; done
got="$(psql "$URL" -Atqc "SELECT v FROM persist" 2>/dev/null || true)"
[ "$got" = "survives" ] || fail "data did not persist across restart (got '$got')"
echo "PASS: Docker image builds, initialises, serves over TCP, and persists across a restart"
