# PGX release

A self-contained, relocatable, PostgreSQL 18 compatible runtime for local
development, CI and disposable databases. No system PostgreSQL required.

## Install

```sh
curl -fsSL https://pgx.dev/install | sh
pgx init && pgx start
pgx create myapp && pgx url myapp
```

Or download the tarball for your platform below, verify its SHA-256 against
`checksums.txt`, and extract it; `bin/pgx` is relocatable.

## Platforms

| Platform | Artifact |
|---|---|
| Linux x86-64 | `pgx-<version>-linux-amd64.tar.gz` |
| Linux arm64  | `pgx-<version>-linux-arm64.tar.gz` |
| macOS arm64  | `pgx-<version>-darwin-arm64.tar.gz` |

XML functions and ICU collations use the host's libxml2 and ICU when present.

## Verified in this release

See `docs/pgx/local-dev/validation-report.md` for the full matrix (unit,
lifecycle, SQL, compatibility vs PostgreSQL 18.6, crash recovery, isolation,
storage, concurrency, stress, and the application fixtures) and the
GO / NO-GO / BLOCKED decision.
