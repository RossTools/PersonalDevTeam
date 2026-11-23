// The cloud bootstrap scripts the site serves (public/cloud/; the docs' Cloud instances page): the cloud-init template
// is valid cloud-config that runs linux.sh with every setting it reads, and both scripts take the same settings and
// end by joining the project. shellcheck and PSScriptAnalyzer lint the scripts themselves (CI's site job).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse } from 'yaml';

const read = (name) => readFileSync(new URL(`../public/cloud/${name}`, import.meta.url), 'utf8');
const SETTINGS = ['TS_AUTHKEY', 'PDT_OWNER_LOGIN', 'PDT_OWNER_ADDRESS', 'PDT_PAIRING_CODE', 'PDT_INSTANCE_NAME', 'PDT_AGENTS', 'GODOT_VERSION'];

test('the cloud-init template is cloud-config that runs linux.sh with its settings', () => {
  const text = read('cloud-init.yaml');
  assert.ok(text.startsWith('#cloud-config\n'), 'cloud-init reads it only with its header');
  const doc = parse(text);
  const env = doc.write_files.find((f) => f.path === '/etc/pdt-bootstrap.env');
  assert.equal(env.permissions, '0600');
  const keys = env.content.trim().split('\n').map((l) => l.split('=')[0]);
  assert.deepEqual(keys, SETTINGS);
  const run = doc.runcmd.flat().join(' ');
  assert.match(run, /curl -fsSL https:\/\/pdt\.dev\/cloud\/linux\.sh \| sh/);
  assert.match(run, /rm -f \/etc\/pdt-bootstrap\.env/, 'the auth key and code go once used');
});

test('both scripts read the same settings, set the owner and join the project', () => {
  const sh = read('linux.sh');
  const ps1 = read('windows.ps1');
  for (const s of SETTINGS) {
    assert.ok(sh.includes(s), `linux.sh doesn't read ${s}`);
    assert.ok(ps1.includes(`$env:${s}`), `windows.ps1 doesn't read ${s}`);
  }
  for (const script of [sh, ps1]) {
    assert.match(script, /api\/host\/settings/);
    assert.match(script, /api\/remote-projects/);
    assert.match(script, /site ?= ?["']https:\/\/pdt\.dev["']/);
    assert.match(script, /\$site\/install\.(sh|ps1)/, 'PDT itself comes from the one-line install');
  }
  assert.ok(!/tailscale up[^\n]*--advertise-tags/.test(sh + ps1), 'the instance keeps your Tailscale identity');
});
