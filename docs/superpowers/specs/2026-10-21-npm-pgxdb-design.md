# `npx pgxdb` — one-command npm distribution for PGX

Date: 2026-10-21. Status: implemented in this branch.

## Intent

`npx pgxdb@latest` must take a developer (or an agent) from nothing to a
running PGX runtime with an `app` database and a printed connection URL, with
no Docker, Rust or system PostgreSQL. It is a thin wrapper around the existing
`pgx` CLI shipped in the GitHub release tarballs; the database engine is not
touched.

## Goals

- `npx pgxdb` (no arguments) behaves exactly like `pgx run`.
- `npx pgxdb <anything>` runs `pgx <anything>` (every command, `--json`
  included), with stdio inherited and the exit code propagated.
- The native distribution is downloaded once per version, SHA-256 verified
  against the release's `.sha256`, and cached under `~/.pgx/dist/<version>`
  (the same place `curl -fsSL https://pgx.dev/install | sh` installs to).
- Platforms: macOS arm64/x64, Linux arm64/x64. Windows prints a clear error.
- Publishing to npm is automated from GitHub Actions when a GitHub release is
  published.

## Non-goals

- No rewrite or vendoring of the engine; no per-platform npm packages (the
  tarballs are ~30 MB each; downloading one on first use is cheaper for
  `npx` than installing four optional dependencies).
- No proxy configuration layer (Node's core `http`/`https` client is used as-is; not `fetch`, whose bundled undici can abort the process when a CDN closes the connection mid-stream).
- No Windows support (PGX has none).

## Decisions (made by the implementer, open to change)

- **Package name `pgxdb`** — `pgx` is taken on npm. Bins: `pgxdb` and
  `pgx`, so `npm i -g pgxdb` also gives a plain `pgx` on PATH.
- **Version pinning.** The npm package version equals the PGX release it
  installs (`pgxdb@0.1.0` → `pgx-0.1.0-<os>-<arch>.tar.gz`). `@latest`
  therefore means "latest published PGX release", and the wrapper and binary
  are always a tested pair. `PGX_VERSION` overrides the binary version.
- **Location in the repo:** `dist/npm/` beside `dist/install.sh` and
  `dist/pgx.dev/`. Zero runtime dependencies, CommonJS, Node >= 18.
- **Cache layout** mirrors the shell installer: `~/.pgx/dist/<version>/`,
  override with `PGX_INSTALL_DIR`. A cached install is valid when
  `bin/pgx` is executable and `share/pgx/dist.json` reports that version;
  anything else is re-downloaded. Install is atomic: extract into a sibling
  `.tmp` directory, then rename into place.
- **Download base** `PGX_BASE_URL` (default GitHub releases) so tests and
  CI can serve a local tarball; this is the same knob `install.sh` has.
- **Extraction** uses the system `tar` (present on every supported OS).
- **Progress** goes to stderr only, so `--json` output stays clean.
- **Publishing:** `.github/workflows/npm-publish.yml` runs on
  `release: published` (and `workflow_dispatch` with a version). It checks
  that the three required release assets exist, stamps the package version
  from the tag, runs the package tests, and publishes with provenance.
  Prereleases are published under the `beta` dist-tag so `@latest` stays
  stable. Auth: npm trusted publishing (OIDC) when configured; `NPM_TOKEN`
  secret as a fallback. The very first publish of the name goes through npm
  staged publishing (`workflow_dispatch` with `stage=true` and a stage-only
  granular token): npm cannot configure a trusted publisher for a package
  that does not exist, and bypass-2FA tokens are deprecated (direct publish
  with them ends around January 2027). A maintainer approves the staged
  version on npmjs.com with 2FA; after that, trusted publishing is configured
  for the package and the token deleted.
  See the next-but-one bullet for the approval gates.
- **macOS x64 (Intel): not supported yet.** No binary is published. The
  wrapper maps `darwin/x64` to `darwin-amd64` and, when the asset is missing,
  says so explicitly. The release matrix gains an experimental
  `macos-15-intel` leg marked `continue-on-error`; its output stays an actions
  artifact and is removed before the draft release is created, so it cannot
  reach users until someone validates it and lifts that step.
- **Publishing cannot happen by accident.** The publish job runs only in
  `pgrundev/pgx`, only for a published (non-draft) release whose tag is
  `vX.Y.Z[-pre]` and whose commit is on `main`, after the verify job passed,
  and inside the `npm-publish` GitHub environment (configure required
  reviewers there to add a human approval step). Manual runs default to
  `dry_run: true`. After a real publish, a clean `ubuntu:24.04` container with
  no PostgreSQL or Docker runs `npx pgxdb@<version>` from the registry.

## Testing

- `dist/npm/test/` (node:test, no network): platform mapping, asset naming,
  checksum parsing, cache validity, and an end-to-end run against a local
  HTTP server serving a fake tarball (download → verify → extract → run,
  cache hit on the second run, tampered checksum refused, 404 reported).
- `dist/test/npm_test.sh`: the real assembled distribution served locally;
  `node dist/npm/bin/pgxdb.js` must start PGX and print a URL, then `stop`.
  Wired into the CI `cli` job after the installer tests.
