// The download buttons link straight to the newest stable release's files, by names the release keeps from one version
// to the next: the bundle action names them, and hosts pick them by the same rules (pdt_core::release::select_asset).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { downloadUrl, REPO } from './links.mjs';

const bundle = readFileSync(new URL('../../.github/actions/bundle/action.yml', import.meta.url), 'utf8');

test('each system downloads its file from releases/latest/download', () => {
  const name = (os) => {
    const url = downloadUrl(os);
    assert.ok(url.startsWith(`${REPO}/releases/latest/download/`), url);
    return url.slice(url.lastIndexOf('/') + 1);
  };
  assert.match(name('mac'), /.\.app\.tar\.gz$/);
  assert.match(name('windows'), /.-setup\.exe$/);
  assert.equal(name('linux'), 'pdt-linux-x86_64.AppImage');
});

test('the release names the Windows installer and the AppImage without their version', () => {
  assert.ok(bundle.includes(`"$RUNNER_TEMP/release/${downloadUrl('windows').split('/').pop()}"`), 'the Windows installer');
  assert.ok(bundle.includes('"$RUNNER_TEMP/release/pdt-linux-$PDT_ARCH.AppImage"'), 'the AppImage');
});
