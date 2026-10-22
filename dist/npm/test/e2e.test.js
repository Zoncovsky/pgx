'use strict';
// End-to-end tests for the pgxdb wrapper against a LOCAL http server serving a
// fake PGX tarball (no network, no real pgx). The fake `pgx` is a shell script
// that echoes its arguments, so these tests prove: download → checksum verify →
// extract → run, the cache hit on the second run, argument pass-through, exit
// code propagation, and refusal of a tampered or missing download.
const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');

const BIN = path.join(__dirname, '..', 'bin', 'pgxdb.js');
const VERSION = '9.9.9-test';
const platform = require('../lib/pgxdb.js');
const TARGET = platform.platformTarget(process.platform, process.arch);
const NAME = `pgx-${VERSION}-${TARGET}`;

function makeTarball(root, version, target) {
  const dist = path.join(root, `pgx-${version}-${target}`);
  fs.mkdirSync(path.join(dist, 'bin'), { recursive: true });
  fs.mkdirSync(path.join(dist, 'share', 'pgx'), { recursive: true });
  fs.writeFileSync(
    path.join(dist, 'bin', 'pgx'),
    '#!/bin/sh\necho "fake-pgx args: $*"\nif [ "$1" = exit-code ]; then exit "$2"; fi\nexit 0\n',
    { mode: 0o755 }
  );
  fs.writeFileSync(path.join(dist, 'share', 'pgx', 'dist.json'), JSON.stringify({ pgx_version: version, os: 'x', arch: 'y' }));
  const tgz = path.join(root, `pgx-${version}-${target}.tar.gz`);
  const r = spawnSync('tar', ['-czf', tgz, '-C', root, `pgx-${version}-${target}`]);
  assert.equal(r.status, 0, r.stderr.toString());
  const sum = crypto.createHash('sha256').update(fs.readFileSync(tgz)).digest('hex');
  return { tgz, sum };
}

/** Serve files from `dir` under /v<version>/; count requests per path. */
function startServer(dir) {
  const hits = {};
  const overrides = {};
  const server = http.createServer((req, res) => {
    hits[req.url] = (hits[req.url] || 0) + 1;
    if (overrides[req.url] !== undefined) {
      res.writeHead(200);
      res.end(overrides[req.url]);
      return;
    }
    const file = path.join(dir, req.url);
    if (!fs.existsSync(file)) {
      res.writeHead(404);
      res.end('not found');
      return;
    }
    res.writeHead(200, { 'content-length': fs.statSync(file).size });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, hits, overrides, url: `http://127.0.0.1:${server.address().port}` });
    });
  });
}

/** Run the wrapper asynchronously: the test's own HTTP server must stay responsive. */
function runPgxdb(args, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [BIN, ...args], { env: { PATH: process.env.PATH, ...env } });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('error', reject);
    child.on('close', (status) => resolve({ status, stdout, stderr }));
  });
}

test('pgxdb end to end', async (t) => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'pgxdb-e2e-'));
  const srvRoot = path.join(tmp, 'srv', `v${VERSION}`);
  const stage = path.join(tmp, 'stage');
  const { tgz, sum } = makeTarball(stage, VERSION, TARGET);
  fs.mkdirSync(srvRoot, { recursive: true });
  fs.copyFileSync(tgz, path.join(srvRoot, `${NAME}.tar.gz`));
  fs.writeFileSync(path.join(srvRoot, `${NAME}.tar.gz.sha256`), `${sum}  ${NAME}.tar.gz\n`);
  const srv = await startServer(path.join(tmp, 'srv'));
  const home = path.join(tmp, 'home');
  const env = { HOME: home, PGX_VERSION: VERSION, PGX_BASE_URL: srv.url };
  const installDir = path.join(home, '.pgx', 'dist', VERSION);
  const tgzPath = `/v${VERSION}/${NAME}.tar.gz`;
  t.after(() => {
    srv.server.close();
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  await t.test('first run downloads, verifies, installs and runs `pgx run` by default', async () => {
    const r = await runPgxdb([], env);
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /^fake-pgx args: run\n$/);
    assert.match(r.stderr, /downloading PGX 9\.9\.9-test/);
    assert.match(r.stderr, /installed PGX 9\.9\.9-test to /);
    assert.equal(srv.hits[tgzPath], 1);
    assert.equal(srv.hits[`${tgzPath}.sha256`], 1);
    assert.ok(fs.existsSync(path.join(installDir, 'bin', 'pgx')), 'bin/pgx installed in ~/.pgx/dist/<version>');
    assert.deepEqual(
      fs.readdirSync(path.join(home, '.pgx', 'dist')).filter((n) => n.startsWith('.')),
      [],
      'no temp dirs left behind'
    );
  });

  await t.test('second run uses the cache and passes arguments through', async () => {
    const r = await runPgxdb(['ls', '--json'], env);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout, 'fake-pgx args: ls --json\n');
    assert.equal(r.stderr, '', 'nothing downloaded, nothing logged');
    assert.equal(srv.hits[tgzPath], 1, 'tarball not fetched again');
  });

  await t.test('a bare --json still runs `run`', async () => {
    const r = await runPgxdb(['--json'], env);
    assert.equal(r.stdout, 'fake-pgx args: --json run\n');
  });

  await t.test('the pgx exit code is propagated', async () => {
    const r = await runPgxdb(['exit-code', '7'], env);
    assert.equal(r.status, 7);
  });

  await t.test('PGX_INSTALL_DIR overrides the cache location', async () => {
    const dir = path.join(tmp, 'custom');
    const r = await runPgxdb(['version'], { ...env, PGX_INSTALL_DIR: dir });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(fs.existsSync(path.join(dir, 'bin', 'pgx')));
    assert.equal(srv.hits[tgzPath], 2);
  });

  await t.test('a cached install with the wrong version is replaced', async () => {
    fs.writeFileSync(path.join(installDir, 'share', 'pgx', 'dist.json'), JSON.stringify({ pgx_version: '0.0.1' }));
    // rename() onto a non-empty dir fails on POSIX, so pgxdb must move the
    // stale dir aside itself and leave no `.old` copy behind.
    const r = await runPgxdb(['version'], env);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(srv.hits[tgzPath], 3, 're-downloaded');
    assert.equal(JSON.parse(fs.readFileSync(path.join(installDir, 'share', 'pgx', 'dist.json'), 'utf8')).pgx_version, VERSION);
    assert.deepEqual(fs.readdirSync(path.join(home, '.pgx', 'dist')), [VERSION], 'no .old or temp dirs left');
  });

  await t.test('a tampered download is refused and nothing is installed', async () => {
    srv.overrides[`${tgzPath}.sha256`] = `${'0'.repeat(64)}  ${NAME}.tar.gz\n`;
    const dir = path.join(tmp, 'bad');
    const r = await runPgxdb(['version'], { ...env, PGX_INSTALL_DIR: dir });
    delete srv.overrides[`${tgzPath}.sha256`];
    assert.equal(r.status, 1);
    assert.match(r.stderr, /checksum mismatch/);
    assert.equal(r.stdout, '');
    assert.ok(!fs.existsSync(dir), 'no partial install');
  });

  await t.test('a missing release asset is reported with the URL and status', async () => {
    const r = await runPgxdb(['version'], { ...env, PGX_VERSION: '0.0.0-nope' });
    assert.equal(r.status, 1);
    assert.match(r.stderr, /download failed: .*pgx-0\.0\.0-nope-.*\.tar\.gz: HTTP 404/);
  });

  await t.test('an unreachable server is reported, not thrown as a stack trace', async () => {
    const r = await runPgxdb(['version'], { ...env, PGX_BASE_URL: 'http://127.0.0.1:1', PGX_VERSION: '0.0.0-down' });
    assert.equal(r.status, 1);
    assert.match(r.stderr, /^pgxdb: download failed: http:\/\/127\.0\.0\.1:1\//m);
    assert.doesNotMatch(r.stderr, /at .*\.js:\d+/);
  });
});
