#!/bin/sh
# Container entrypoint: run the PGX engine in the foreground as PID 1.
# First run initialises the cluster on the volume (reading PGX_PASSWORD,
# PGX_PORT, PGX_LISTEN, PGX_DURABILITY); later runs reuse it.
set -eu
mkdir -p "$PGX_HOME" "$PGX_RUN_DIR"
if [ ! -f "$PGX_HOME/config.json" ]; then
  echo "pgx: first run — initialising cluster in $PGX_HOME (listen ${PGX_LISTEN}:${PGX_PORT})"
  pgx init --port "${PGX_PORT:-5432}" ${PGX_DURABILITY:+--durability "$PGX_DURABILITY"} >/dev/null
  if [ -z "${PGX_PASSWORD:-}" ]; then
    echo "pgx: no PGX_PASSWORD set; a random superuser password was generated (run 'pgx url postgres' inside the container to see the connection URL)"
  fi
fi
exec pgx serve
