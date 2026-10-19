// Cloudflare Worker for https://pgx.dev/install
//
// Serves the reviewed, version-controlled dist/install.sh so that
//   curl -fsSL https://pgx.dev/install | sh
// works. The script body is fetched from this repository at a PINNED ref
// (a release tag or commit), never a moving branch, so what runs on a user's
// machine is exactly what was reviewed and tagged. Everything else on pgx.dev
// is left untouched (the Worker only owns the /install route).
//
// Deploy: see README.md in this directory. Set INSTALL_REF to the released
// tag (e.g. v0.1.0-beta.1) at deploy time. NOT yet deployed — pending approval
// and Cloudflare access.
const REPO = "pgrundev/pgx";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/install") {
      return new Response("not found", { status: 404 });
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method not allowed", { status: 405 });
    }
    const ref = (env && env.INSTALL_REF) || "main";
    const raw = `https://raw.githubusercontent.com/${REPO}/${ref}/dist/install.sh`;
    const upstream = await fetch(raw, { cf: { cacheTtl: 300, cacheEverything: true } });
    if (!upstream.ok) {
      // Fail loudly: never serve an empty/200 body that would make `| sh` a no-op.
      return new Response(`pgx installer unavailable (upstream ${upstream.status})\n`, {
        status: 502,
        headers: { "content-type": "text/plain; charset=utf-8" },
      });
    }
    const body = await upstream.text();
    return new Response(body, {
      status: 200,
      headers: {
        "content-type": "text/x-shellscript; charset=utf-8",
        "cache-control": "public, max-age=300",
        "x-pgx-install-ref": ref,
      },
    });
  },
};
