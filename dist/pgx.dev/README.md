# pgx.dev `/install` endpoint (prepared, not deployed)

`https://pgx.dev/install` must serve the reviewed [`dist/install.sh`](../install.sh)
so that `curl -fsSL https://pgx.dev/install | sh` works. Today `pgx.dev` is a
Cloudflare-hosted placeholder page and `/install` returns **HTTP 404**.

## Current hosting (identified)
- `pgx.dev` resolves through **Cloudflare** (response headers: `server: cloudflare`, `cf-ray`, `cf-cache-status`).
- The root currently serves a **parked/placeholder page** (an `<x-dc>` document + `support.js`), not a managed site. There is **no** `/install` route.
- The serving configuration lives in the **Cloudflare account**, not in this repository.

## What this directory provides
A Cloudflare **Worker** (`install-worker.js` + `wrangler.toml`) that owns only the
`pgx.dev/install` route and returns `dist/install.sh` from a **pinned ref**
(a release tag), with `content-type: text/x-shellscript`. It fails with `502`
(never a misleading empty `200`) if the upstream script can't be fetched.

## Deploy (maintainer, after approval — needs Cloudflare access)
```sh
cd dist/pgx.dev
npx wrangler deploy --var INSTALL_REF:v0.1.0-beta.1   # pin to the released tag
curl -fsSL https://pgx.dev/install | head             # verify it serves the script
```
This adds a single route; it does **not** change DNS or other pgx.dev content.

## Prerequisites before the public `curl | sh` works
1. Publish the GitHub release (tag `v0.1.0-beta.1`) with the platform tarballs + `.sha256` (via `.github/workflows/release.yml`, draft → manual publish).
2. Deploy this Worker with `INSTALL_REF` set to that tag.
3. `dist/install.sh` then resolves the release artifacts (its discovery reads the releases list, which includes prereleases).
