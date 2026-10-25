# `pgx copy` — PostgreSQL → PGX import, design

Date: 2026-10-24. Status: implemented on branch `feat/pgx-copy`.

## Intent

A developer who already runs PostgreSQL copies one database into the local PGX
runtime once, then branches it per agent with the existing `pgx clone`. The
source is never modified; a half-imported database is never visible under the
requested name.

```
pgx copy --from "$DATABASE_URL" --to myapp_template [--schema-only] [--jobs N] [--verbose] [--no-optimize]
pgx clone myapp_template agent-1 --ttl 2h
pgx url agent-1
```

## Findings that shaped the design

- The CLI (`crates/bin/pgx`) talks to the engine only over the Unix socket with
  trust auth, through a ~500-line built-in wire client (`pgwire.rs`) that has
  no SCRAM and no TLS. Connecting to an arbitrary external PostgreSQL from Rust
  would mean implementing SCRAM-SHA-256 and TLS or adding heavy dependencies.
- The distribution bundles no PostgreSQL client tools; it is built on a machine
  that has PostgreSQL 18 for `initdb` only. Bundling `pg_dump`/`pg_restore`
  means bundling libpq + OpenSSL for three platforms: out of scope for an MVP.
- Workflow A's premise is "the developer already has PostgreSQL locally", which
  ships `pg_dump`, `pg_restore` and `psql` together.
- Verified on the real engine (v0.1.0, APFS): `pg_restore` of a custom-format
  dump over the Unix socket restores extensions, types, functions, triggers,
  views, materialized views, comments, constraints and data; `ALTER DATABASE …
  RENAME` works; `pgx clone` of the result is a block clone (64 ms).
- `docs/pgx/compatibility-gate.md` names `pg_dump --schema-only` restored into a
  scratch PGX database as the authoritative compatibility test, with extension
  and language allowlists as cheap pre-checks.

## Decisions

1. **Export/import tool: external `pg_dump` / `pg_restore`** (option 3 of the
   brief), auto-detected: `PGX_PG_BINDIR`, then `PATH`, then well-known
   Homebrew / Postgres.app / Debian / RHEL directories; the highest version
   found wins. Missing tools are a clear error with per-OS install hints, and
   `pgx doctor` reports them. No Rust port, no bundling, in this release.
2. **No source connection from Rust.** Everything the CLI learns about the
   source comes from `pg_dump` and the archive's table of contents
   (`pg_restore -l`): server version, extensions, procedural languages,
   object inventory, publications. Credentials never cross the wire client.
3. **Credentials:** the `--from` value is parsed (URL or `key=value` conninfo),
   the password is removed from what is passed as an argument and handed to the
   child only through `PGPASSWORD`. Every byte of child output shown to the
   user passes through a redactor that masks the password. URLs without a
   password keep working through the user's own `PGPASSWORD` / `~/.pgpass`.
4. **Two dumps, one snapshot each.** A fast `--schema-only` dump first (the
   compatibility gate runs on its TOC before any data moves), then one
   full custom-format dump (a single snapshot, consistent under concurrent
   traffic) that is restored in TOC order. With `--schema-only` the first dump
   is the only dump.
5. **Compatibility gate before restore:** extensions in the TOC must exist in
   the engine's `pg_available_extensions`; `PROCEDURAL LANGUAGE` entries must be
   `plpgsql`/`sql`; subscriptions/event triggers are refused; publications are
   restored with `--no-publications` and reported as warnings by name; source
   major > 18 is a warning. The restore itself, with `--exit-on-error`, is the
   final arbiter: any failing object aborts the import (no best-effort mode).
6. **Staging + atomic promotion.** Restore into `_pgxcopy_<dest>_<rand>`; on
   success run `VACUUM (FREEZE, ANALYZE)` (sets hint bits so a clone's first
   read does not un-share its pages) and `ALTER DATABASE … RENAME TO <dest>`
   (atomic in the catalog). On any failure drop the staging database and the
   temp dir. A marker file `$PGX_HOME/copy/<staging>.json` (pid, destination)
   lets the next `pgx copy` sweep staging databases left by a killed process,
   and lets `pgx ls` show them as `importing`.
7. **Destination safety:** the destination must not exist (exit 4), must pass
   the CLI's name rules and must not be a system database. No overwrite flag.
8. **Validation:** restore exit status, the TOC inventory against the staging
   catalog (tables, sequences, views, materialized views, functions exact;
   indexes at least as many), and a connectivity query. No row-count comparison
   against the source (no source connection); documented.
9. **Temp storage:** `$PGX_HOME/tmp/copy-<pid>-<rand>/` mode 0700, removed on
   success, failure and SIGINT (handler sets a flag; children are terminated).
10. **Not in this release:** `--data-only`, `pgx refresh`, bundled client tools,
    overwrite. The staging/promote split is what a later `pgx refresh` needs:
    import into staging, then swap names.

## Exit codes and JSON

`2` bad URL/name/flags · `3` PGX not initialised or not running · `4`
destination exists · `1` everything else (tools missing, source unreachable,
auth failed, incompatible, dump/restore failed, interrupted). `--json` prints
one object (`status`, `operation`, `source`, `destination`, `schema_only`,
`stages`, `warnings`, `duration_ms`) on success and the CLI's standard
`{"error":{…}}` on failure, with a `details` object for structured causes
(for example `unsupported_extensions`).
