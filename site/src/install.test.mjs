// public/install.sh against a fake release: GitHub's API (PDT_INSTALL_API) and github.com's release downloads
// (PDT_INSTALL_WEB) are a local HTTP server answering from a map, HOME is a temp folder, and uname, getconf, loginctl, systemctl, open and codesign are stubs on PATH (codesign passes
// unless $HOME/codesign.fail holds its message; systemctl has a user instance unless `systemd: false`). The Linux
// tarball's pdt-host is a stub that records how it was called (and fails when $HOME/host.fail exists), so nothing is
// registered for real, and no running PDT is looked for on 7420.
import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { createServer } from 'node:http';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../public/install.sh', import.meta.url));
// A POSIX shell script: not on Windows (install.ps1 is Windows').
const skip = process.platform === 'win32' && 'install.sh needs a POSIX shell';
const REPO = 'RossTools/PersonalDevTeam';
const APP = 'Personal Dev Team.app';

function stub(dir, name, body) {
  writeFileSync(join(dir, name), `#!/bin/sh\n${body}\n`);
  chmodSync(join(dir, name), 0o755);
}

// The fake GitHub: path and query → body, `{ redirect }` (a 302 there) or `{ status }`. Each test's paths are under
// its own /<n>/: the API's under /<n>/repos, github.com's under /<n>/web. `requests` lists every path asked for.
const routes = new Map();
const requests = [];
const server = createServer((req, res) => {
  requests.push(req.url);
  const route = routes.get(req.url);
  if (route?.redirect) {
    res.writeHead(302, { Location: route.redirect });
    res.end();
  } else if (route?.status) {
    res.writeHead(route.status);
    res.end('{"message":"API rate limit exceeded"}');
  } else {
    res.writeHead(route === undefined ? 404 : 200);
    res.end(route ?? '{"message":"Not Found"}');
  }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
after(() => server.close());
let tests = 0;
let assetIds = 100;

/** A temp HOME, stubs for `uname -s`/`uname -m` = os/arch and glibc `glibc`, and an empty fake API. */
function setup({ os = 'Linux', arch = 'x86_64', glibc = '2.35', systemd = true } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'pdt-install-test-'));
  const home = join(root, 'home');
  const bin = join(root, 'bin');
  const api = `/${++tests}`;
  for (const d of [home, bin]) mkdirSync(d, { recursive: true });
  stub(bin, 'uname', `case "$1" in -s) echo ${os} ;; -m) echo ${arch} ;; *) echo ${os} ;; esac`);
  stub(bin, 'getconf', glibc ? `echo "glibc ${glibc}"` : 'exit 1');
  stub(bin, 'loginctl', `echo "$*" >> "$HOME/loginctl.log"; case "$1" in show-user) echo Linger=no ;; esac`);
  stub(bin, 'systemctl', systemd ? `echo "$*" >> "$HOME/systemctl.log"` : 'echo "Failed to connect to bus" >&2; exit 1');
  stub(bin, 'open', `echo "$*" >> "$HOME/open.log"`);
  stub(bin, 'codesign', `echo "$*" >> "$HOME/codesign.log"; if [ -f "$HOME/codesign.fail" ]; then cat "$HOME/codesign.fail" >&2; exit 1; fi`);
  return { root, home, bin, api, cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

/** A release signing key: `key` as install.sh pins it (base64 SPKI DER), `sig(tag, sums)` SHA256SUMS.sig as the
 * release workflow makes it (DER, over "<tag>\n" + SHA256SUMS). */
function releaseKey() {
  const { publicKey, privateKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  return {
    key: publicKey.export({ type: 'spki', format: 'der' }).toString('base64'),
    sig: (tag, sums) => sign('sha256', Buffer.from(`${tag}\n${sums}`), { key: privateKey, dsaEncoding: 'der' }),
  };
}

/** Publishes a release with `files` (name → contents) and SHA256SUMS (from `sums`, or computed), as GitHub's API would
 * list it (`latest`, and `tags/<tag>`) and github.com would serve it (releases/latest redirecting to its tag when
 * `latest`, and each file under releases/download/<tag>/). With `signedBy` (a `releaseKey()`), SHA256SUMS.sig too, signed for `signedTag`
 * (default `tag`). */
function publish(t, tag, files, { sums, pretty = true, latest = true, signedBy, signedTag = tag } = {}) {
  const rel = `${t.api}/repos/${REPO}/releases`;
  const web = `${t.api}/web/${REPO}/releases`;
  const all = { ...files };
  all.SHA256SUMS = sums ?? Object.entries(files).map(([n, b]) => `${createHash('sha256').update(b).digest('hex')}  ${n}\n`).join('');
  if (signedBy) all['SHA256SUMS.sig'] = signedBy.sig(signedTag, all.SHA256SUMS);
  const assets = Object.entries(all).map(([name, body]) => {
    const id = assetIds++;
    routes.set(`${rel}/assets/${id}`, body);
    routes.set(`${web}/download/${tag}/${name}`, body);
    return {
      url: `https://api.github.com/repos/${REPO}/releases/assets/${id}`,
      id,
      name,
      uploader: { login: 'bot', url: 'https://api.github.com/users/bot' },
      browser_download_url: `https://github.com/${REPO}/releases/download/${tag}/${name}`,
    };
  });
  const json = JSON.stringify({ url: `https://api.github.com/repos/${REPO}/releases/1`, tag_name: tag, name: tag, assets }, null, pretty ? 2 : 0);
  routes.set(`${rel}/tags/${tag}`, json);
  if (latest) {
    routes.set(`${rel}/latest`, json);
    routes.set(`${web}/latest`, { redirect: `http://127.0.0.1:${server.address().port}${web}/tag/${tag}` });
  }
}

/** The paths of `t`'s API requests (not github.com's). */
const apiCalls = (t) => requests.filter((p) => p.startsWith(`${t.api}/repos/`));

/** A gzipped tar of `entries` (path → contents; executable when the path ends in pdt-host). */
function tarball(t, entries) {
  const dir = mkdtempSync(join(t.root, 'tar-'));
  for (const [path, body] of Object.entries(entries)) {
    mkdirSync(join(dir, path, '..'), { recursive: true });
    writeFileSync(join(dir, path), body);
    if (path.endsWith('pdt-host')) chmodSync(join(dir, path), 0o755);
  }
  const out = join(t.root, `${Math.random().toString(36).slice(2)}.tar.gz`);
  const top = [...new Set(Object.keys(entries).map((p) => p.split('/')[0]))];
  const r = spawnSync('tar', ['-czf', out, '-C', dir, ...top]);
  assert.equal(r.status, 0, String(r.stderr));
  return readFileSync(out);
}

const hostStub = '#!/bin/sh\necho "$*" >> "$HOME/host.log"\nif [ -f "$HOME/host.fail" ]; then echo "systemctl --user daemon-reload failed" >&2; exit 1; fi\n';
const linuxTarball = (t) => tarball(t, { 'pdt/pdt-host': hostStub, 'pdt/pdtd': 'daemon', 'pdt/web/index.html': '<html>' });

/** Runs the script (asynchronously: the fake API answers from this process) with `extraEnv`. No PDT is looked for
 * on 7420: one answers on the fake API when `running`. */
function run(t, args = [], extraEnv = {}, { running = false } = {}) {
  const { port } = server.address();
  const local = `http://127.0.0.1:${port}${running ? `${t.api}/pdt` : '/no-pdt-running'}`;
  if (running) routes.set(`${t.api}/pdt/api/health`, '{"name":"Personal Dev Team","version":"0.3.0"}');
  const env = { PATH: `${t.bin}:${process.env.PATH}`, HOME: t.home, USER: 'tester',
    PDT_INSTALL_API: `http://127.0.0.1:${port}${t.api}`, PDT_INSTALL_WEB: `http://127.0.0.1:${port}${t.api}/web`,
    PDT_INSTALL_LOCAL: local,
    PDT_INSTALL_SYSTEM_APPS: join(t.root, 'system-apps'), ...extraEnv };
  const child = spawn('sh', [script, ...args], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (d) => (stdout += d));
  child.stderr.on('data', (d) => (stderr += d));
  return new Promise((resolve) => child.on('close', (status) => resolve({ status, stdout, stderr })));
}

const read = (path) => (existsSync(path) ? readFileSync(path, 'utf8') : '');

test('installs the Linux tarball, registers it and turns on lingering', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t), 'pdt-linux-aarch64.tar.gz': 'other' });
  const r = await run(t, ['--yes']);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /Checksum verified/);
  assert.match(r.stdout, /v0\.3\.0 is installed/);
  assert.match(r.stdout, /127\.0\.0\.1:7420|no-pdt-running/);
  assert.ok(existsSync(join(t.home, '.local/opt/pdt/pdtd')));
  assert.equal(read(join(t.home, 'host.log')), 'install --no-linger-hint\n');
  assert.match(read(join(t.home, 'loginctl.log')), /enable-linger tester/);
});

test('on a Linux desktop, pdt-host gives the linger hint and the script points at the AppImage for a tray icon', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  const r = await run(t, [], { WAYLAND_DISPLAY: 'wayland-0' });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(read(join(t.home, 'host.log')), 'install\n');
  assert.equal(read(join(t.home, 'loginctl.log')), '');
  assert.match(r.stdout, /no tray icon\. For one, install the AppImage/);
});

test('a folder it cannot write to is refused before downloading', { skip: skip || (process.getuid?.() === 0 && 'root writes anywhere') }, async (ctx) => {
  const t = setup();
  const local = join(t.home, '.local');
  mkdirSync(local);
  chmodSync(local, 0o555);
  ctx.after(() => {
    chmodSync(local, 0o755);
    t.cleanup();
  });
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  const r = await run(t, ['--yes']);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /you can't write to .*\/home\/\.local\. .*Nothing was installed/);
  assert.doesNotMatch(r.stdout, /Downloading/);
  assert.equal(read(join(t.home, 'host.log')), '');
});

test('picks the aarch64 tarball, from compact JSON', { skip }, async (ctx) => {
  const t = setup({ arch: 'arm64' });
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': 'other', 'pdt-linux-aarch64.tar.gz': linuxTarball(t) }, { pretty: false });
  const r = await run(t, ['--yes']);
  assert.equal(r.status, 0, r.stderr);
  assert.ok(existsSync(join(t.home, '.local/opt/pdt/pdtd')));
});

test('an exact version and the nightly channel', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.2.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) }, { latest: false });
  const r = await run(t, ['--yes', '--version', '0.2.0']);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /v0\.2\.0 is installed/);

  const n = setup();
  ctx.after(n.cleanup);
  publish(n, 'v0.3.1-nightly.20261030', { 'pdt-linux-x86_64.tar.gz': linuxTarball(n) }, { latest: false });
  publish(n, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': 'stable' });
  const list = ['v0.3.1-nightly.20261030', 'test-v9.9.9', 'v0.3.0'].map((tag) => ({ tag_name: tag, prerelease: tag !== 'v0.3.0' }));
  routes.set(`${n.api}/repos/${REPO}/releases?per_page=30`, JSON.stringify(list, null, 2));
  const rn = await run(n, ['--yes', '--channel', 'nightly']);
  assert.equal(rn.status, 0, rn.stderr);
  assert.match(rn.stdout, /v0\.3\.1-nightly\.20261030 is installed/);
  assert.match(rn.stdout, /Nightly builds too/);
});

test('installs the app on macOS into ~/Applications and opens it', { skip }, async (ctx) => {
  const t = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(t.cleanup);
  const app = tarball(t, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: '<plist/>' });
  publish(t, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app, 'Personal.Dev.Team_0.3.0_x64-setup.exe': 'exe' });
  const r = await run(t);
  assert.equal(r.status, 0, r.stderr);
  const dest = join(t.home, 'Applications', APP);
  assert.ok(existsSync(join(dest, 'Contents/Info.plist')));
  assert.equal(read(join(t.home, 'open.log')), `${dest}\n`);
  assert.match(read(join(t.home, 'codesign.log')), /^--verify --deep --strict .*Personal Dev Team\.app\n$/);
});

test('an app whose signature does not verify is not installed', { skip }, async (ctx) => {
  const t = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(t.cleanup);
  const app = tarball(t, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: '<plist/>' });
  publish(t, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app });
  writeFileSync(join(t.home, 'codesign.fail'), 'a sealed resource is missing or invalid\n');
  const r = await run(t);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /signature of .* does not verify/);
  assert.match(r.stderr, /a sealed resource is missing or invalid/);
  assert.match(r.stderr, /Nothing was installed/);
  assert.ok(!existsSync(join(t.home, 'Applications', APP)));
  assert.equal(read(join(t.home, 'open.log')), '');
});

test('a checksum mismatch or a missing entry aborts and installs nothing', { skip }, async (ctx) => {
  for (const sums of [`${'0'.repeat(64)}  pdt-linux-x86_64.tar.gz\n`, `${'0'.repeat(64)}  something-else\n`]) {
    const t = setup();
    ctx.after(t.cleanup);
    publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) }, { sums });
    const r = await run(t, ['--yes']);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /does not match SHA256SUMS|does not list/);
    assert.match(r.stderr, /Nothing was installed/);
    assert.ok(!existsSync(join(t.home, '.local/opt/pdt')));
    assert.equal(read(join(t.home, 'host.log')), '');
  }
});

test('a release without SHA256SUMS aborts', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  routes.delete(`${t.api}/web/${REPO}/releases/download/v0.3.0/SHA256SUMS`);
  const r = await run(t, ['--yes']);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /no SHA256SUMS/);
  assert.ok(!existsSync(join(t.home, '.local/opt/pdt')));

  // With a token, the release's asset list says so.
  const k = setup();
  ctx.after(k.cleanup);
  publish(k, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(k) });
  const latest = `${k.api}/repos/${REPO}/releases/latest`;
  const json = JSON.parse(routes.get(latest));
  json.assets = json.assets.filter((a) => a.name !== 'SHA256SUMS');
  routes.set(latest, JSON.stringify(json));
  const rk = await run(k, ['--yes'], { GITHUB_TOKEN: 'token' });
  assert.notEqual(rk.status, 0);
  assert.match(rk.stderr, /no SHA256SUMS/);
  assert.ok(!existsSync(join(k.home, '.local/opt/pdt')));
});

test('without a token, a stable install calls no API and a nightly one calls it once', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  const r = await run(t, ['--yes']);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(apiCalls(t), []);
  assert.ok(requests.includes(`${t.api}/web/${REPO}/releases/download/v0.3.0/pdt-linux-x86_64.tar.gz`));

  const v = setup();
  ctx.after(v.cleanup);
  publish(v, 'v0.2.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(v) }, { latest: false });
  assert.equal((await run(v, ['--yes', '--version', '0.2.0'])).status, 0);
  assert.deepEqual(apiCalls(v), []);

  const n = setup();
  ctx.after(n.cleanup);
  publish(n, 'v0.3.1-nightly.20261030', { 'pdt-linux-x86_64.tar.gz': linuxTarball(n) }, { latest: false });
  routes.set(`${n.api}/repos/${REPO}/releases?per_page=30`, JSON.stringify([{ tag_name: 'v0.3.1-nightly.20261030' }]));
  assert.equal((await run(n, ['--yes', '--channel', 'nightly'])).status, 0);
  assert.deepEqual(apiCalls(n), [`${n.api}/repos/${REPO}/releases?per_page=30`]);
});

test('with a token, the release and its files come through the API', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  for (const key of [...routes.keys()]) if (key.startsWith(`${t.api}/web/`)) routes.delete(key);
  const r = await run(t, ['--yes'], { GITHUB_TOKEN: 'token' });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /v0\.3\.0 is installed/);
  assert.ok(apiCalls(t).some((p) => p.includes('/releases/assets/')));
});

test('a failed download says why', { skip }, async (ctx) => {
  const cases = [
    [{ status: 403 }, /could not download pdt-linux-x86_64\.tar\.gz of v0\.3\.0\. .*rate limit may be used up.*GITHUB_TOKEN/],
    [{ status: 500 }, /could not download pdt-linux-x86_64\.tar\.gz of v0\.3\.0\. GitHub answered HTTP 500/],
    [undefined, /release v0\.3\.0 has no download for this system \(pdt-linux-x86_64\.tar\.gz\)/],
  ];
  for (const [answer, message] of cases) {
    const t = setup();
    ctx.after(t.cleanup);
    publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
    const file = `${t.api}/web/${REPO}/releases/download/v0.3.0/pdt-linux-x86_64.tar.gz`;
    if (answer) routes.set(file, answer);
    else routes.delete(file);
    const r = await run(t, ['--yes']);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, message);
    assert.ok(!existsSync(join(t.home, '.local/opt/pdt')));
  }
});

test('with a pinned key, only a release whose SHA256SUMS it signed is installed', { skip }, async (ctx) => {
  const release = releaseKey();
  const env = { PDT_INSTALL_SIGNING_KEY: release.key };
  const cases = [
    ['signed', { signedBy: release }, null],
    ['unsigned', {}, /is not signed/],
    ['signed for another release', { signedBy: release, signedTag: 'v0.2.0' }, /does not match .* release key/],
    ['signed by another key', { signedBy: releaseKey() }, /does not match .* release key/],
  ];
  for (const [name, opts, refused] of cases) {
    const t = setup();
    ctx.after(t.cleanup);
    publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) }, opts);
    const r = await run(t, ['--yes'], env);
    if (refused) {
      assert.notEqual(r.status, 0, name);
      assert.match(r.stderr, refused, name);
      assert.match(r.stderr, /Nothing was installed/, name);
      assert.ok(!existsSync(join(t.home, '.local/opt/pdt')), `${name}: nothing installed`);
    } else {
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.stdout, /Signature verified[\s\S]*Checksum verified/);
      assert.ok(existsSync(join(t.home, '.local/opt/pdt/pdtd')));
    }
  }
  // A signed SHA256SUMS whose entry was changed (a swapped asset) fails the signature, not just the checksum.
  const t = setup();
  ctx.after(t.cleanup);
  const tarball = linuxTarball(t);
  const sums = `${createHash('sha256').update(tarball).digest('hex')}  pdt-linux-x86_64.tar.gz\n`;
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': tarball }, { sums, signedBy: { sig: (tag) => release.sig(tag, sums.replace(/^./, (c) => (c === '0' ? '1' : '0'))) } });
  const r = await run(t, ['--yes'], env);
  assert.match(r.stderr, /does not match .* release key/);
});

test('installs the universal app on an Intel Mac', { skip }, async (ctx) => {
  const t = setup({ os: 'Darwin', arch: 'x86_64' });
  ctx.after(t.cleanup);
  const app = tarball(t, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: '<plist/>' });
  publish(t, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app, 'pdt-linux-x86_64.tar.gz': 'other' });
  const r = await run(t);
  assert.equal(r.status, 0, r.stderr);
  assert.ok(existsSync(join(t.home, 'Applications', APP, 'Contents/Info.plist')));
});

test('unsupported systems are refused with a clear reason', { skip }, async (ctx) => {
  const cases = [
    [{ os: 'FreeBSD' }, /runs on macOS, Linux and Windows, not FreeBSD/],
    [{ arch: 'i686' }, /x86_64 and aarch64, not i686/],
    [{ glibc: '2.31' }, /glibc 2\.35 or newer.*glibc 2\.31/],
    [{ glibc: '' }, /no glibc/],
    [{ os: 'MINGW64_NT-10.0' }, /install\.ps1/],
  ];
  for (const [opts, message] of cases) {
    const t = setup(opts);
    ctx.after(t.cleanup);
    publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
    const r = await run(t, ['--yes']);
    assert.notEqual(r.status, 0, JSON.stringify(opts));
    assert.match(r.stderr, message);
  }
});

/** An old install's files in ~/.local/opt/pdt, and its systemd unit when `registered`. */
function oldLinuxInstall(t, { registered }) {
  mkdirSync(join(t.home, '.local/opt/pdt'), { recursive: true });
  writeFileSync(join(t.home, '.local/opt/pdt/pdt-host'), 'old');
  if (registered) {
    mkdirSync(join(t.home, '.config/systemd/user'), { recursive: true });
    writeFileSync(join(t.home, '.config/systemd/user/pdt-host.service'), '[Unit]');
  }
}

test('a running install is left alone unless --force', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  oldLinuxInstall(t, { registered: true });
  const r = await run(t, ['--yes'], {}, { running: true });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /already installed in .*, and running/);
  assert.match(r.stdout, /Settings > This host > Updates/);
  assert.equal(read(join(t.home, 'host.log')), '');

  const f = await run(t, ['--yes', '--force'], {}, { running: true });
  assert.equal(f.status, 0, f.stderr);
  assert.equal(read(join(t.home, 'host.log')), 'install --replace --no-linger-hint\n');
  assert.ok(existsSync(join(t.home, '.local/opt/pdt/pdtd')));
});

test('an install that starts at login but is stopped says how to start it', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  oldLinuxInstall(t, { registered: true });
  const r = await run(t, ['--yes']);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /isn't running/);
  assert.match(r.stdout, /systemctl --user start pdt-host\.service/);
  assert.doesNotMatch(r.stdout, /keeps itself up to date/);
  assert.equal(read(join(t.home, 'host.log')), '');

  const m = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(m.cleanup);
  mkdirSync(join(m.home, 'Applications', APP), { recursive: true });
  mkdirSync(join(m.home, 'Library/LaunchAgents'), { recursive: true });
  writeFileSync(join(m.home, 'Library/LaunchAgents/dev.personaldevteam.host.plist'), '<plist/>');
  const rm = await run(m);
  assert.equal(rm.status, 0, rm.stderr);
  assert.match(rm.stdout, /Start it with: open ".*Personal Dev Team\.app"/);
  assert.equal(read(join(m.home, 'open.log')), '');
});

test('files left without a login registration are installed again', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  oldLinuxInstall(t, { registered: false });
  const r = await run(t, ['--yes']);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /isn't set up to start at login/);
  assert.match(r.stdout, /v0\.3\.0 is installed/);
  assert.equal(read(join(t.home, 'host.log')), 'install --no-linger-hint\n');
  assert.ok(existsSync(join(t.home, '.local/opt/pdt/pdtd')));

  // macOS: the app kept after `pdt-host uninstall` is replaced and opened.
  const m = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(m.cleanup);
  const app = tarball(m, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: 'new' });
  publish(m, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app });
  const dest = join(m.home, 'Applications', APP);
  mkdirSync(join(dest, 'Contents'), { recursive: true });
  writeFileSync(join(dest, 'Contents/Info.plist'), 'old');
  const rm = await run(m);
  assert.equal(rm.status, 0, rm.stderr);
  assert.equal(read(join(dest, 'Contents/Info.plist')), 'new');
  assert.equal(read(join(m.home, 'open.log')), `${dest}\n`);
});

test('Linux without a systemd user instance downloads nothing', { skip }, async (ctx) => {
  const t = setup({ systemd: false });
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  const r = await run(t, ['--yes']);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /no systemd user instance/);
  assert.match(r.stderr, /systemd=true/);
  assert.match(r.stderr, /Nothing was installed/);
  assert.doesNotMatch(r.stdout, /Downloading/);
  assert.ok(!existsSync(join(t.home, '.local/opt/pdt')));
});

test('a failed registration says how to finish, and the next run installs again', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  publish(t, 'v0.3.0', { 'pdt-linux-x86_64.tar.gz': linuxTarball(t) });
  writeFileSync(join(t.home, 'host.fail'), '');
  const r = await run(t, ['--yes']);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /registering it as a systemd user service failed/);
  assert.match(r.stderr, /finish the install with: ".*\.local\/opt\/pdt\/pdt-host" install/);
  assert.doesNotMatch(r.stdout, /is installed and starts/);
  const forced = await run(t, ['--yes', '--force']);
  assert.notEqual(forced.status, 0);
  assert.match(forced.stderr, /finish the install with: ".*pdt-host" install --replace/);

  rmSync(join(t.home, 'host.fail'));
  const again = await run(t, ['--yes']);
  assert.equal(again.status, 0, again.stderr);
  assert.doesNotMatch(again.stdout, /already installed/);
  assert.match(again.stdout, /v0\.3\.0 is installed/);
  assert.equal(read(join(t.home, 'host.log')), 'install --no-linger-hint\ninstall --replace --no-linger-hint\ninstall --no-linger-hint\n');
});

test('bad options are refused', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  assert.match((await run(t, ['--channel', 'beta'])).stderr, /stable or nightly/);
  assert.match((await run(t, ['--version', 'latest'])).stderr, /like 0\.3\.0/);
  assert.match((await run(t, ['--bogus'])).stderr, /unknown option/);
});

test('no readable releases (a private repository) asks for GITHUB_TOKEN', { skip }, async (ctx) => {
  const t = setup();
  ctx.after(t.cleanup);
  const r = await run(t, ['--yes']);
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /set GITHUB_TOKEN/);
});

test('--force on macOS replaces the app where it is, but not in a folder only an admin can write', { skip }, async (ctx) => {
  const t = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(() => {
    chmodSync(join(t.root, 'system-apps'), 0o755);
    t.cleanup();
  });
  const app = tarball(t, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: 'new' });
  publish(t, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app });

  const mine = join(t.home, 'Applications', APP, 'Contents');
  mkdirSync(mine, { recursive: true });
  writeFileSync(join(mine, 'Info.plist'), 'old');
  const r = await run(t, ['--force']);
  assert.equal(r.status, 0, r.stderr);
  assert.equal(read(join(mine, 'Info.plist')), 'new');
  assert.ok(!existsSync(join(t.home, 'Applications', `${APP}.old`)));
  rmSync(join(t.home, 'Applications'), { recursive: true });

  const system = join(t.root, 'system-apps');
  mkdirSync(join(system, APP), { recursive: true });
  chmodSync(system, 0o555);
  const s = await run(t, ['--force']);
  assert.notEqual(s.status, 0);
  assert.match(s.stderr, /needs admin rights to replace/);
  assert.ok(existsSync(join(system, APP)));
  assert.ok(!existsSync(join(t.home, 'Applications', APP)));
});

test('an app that is not running in a folder only an admin can write is opened, not replaced', { skip }, async (ctx) => {
  const t = setup({ os: 'Darwin', arch: 'arm64' });
  ctx.after(() => {
    chmodSync(join(t.root, 'system-apps'), 0o755);
    t.cleanup();
  });
  const app = tarball(t, { [`${APP}/Contents/MacOS/pdt-host`]: hostStub, [`${APP}/Contents/Info.plist`]: 'new' });
  publish(t, 'v0.3.0', { 'Personal.Dev.Team.app.tar.gz': app });
  const system = join(t.root, 'system-apps');
  mkdirSync(join(system, APP), { recursive: true });
  chmodSync(system, 0o555);
  const r = await run(t);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /isn't running/);
  assert.match(r.stdout, new RegExp(`Start it with: open ".*system-apps/${APP}"`));
  assert.doesNotMatch(r.stdout + r.stderr, /admin rights|keeps itself up to date/);
  assert.doesNotMatch(r.stdout, /Downloading/);
  assert.ok(!existsSync(join(t.home, 'Applications', APP)));
});
