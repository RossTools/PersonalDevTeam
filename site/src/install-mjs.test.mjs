import { test } from 'node:test';
import assert from 'node:assert/strict';
import { detectOs, installCommand } from './install.mjs';

test('the command for each system, from the site it is served by', () => {
  assert.equal(installCommand('mac', 'https://pdt.dev/'), 'curl -fsSL https://pdt.dev/install.sh | sh');
  assert.equal(installCommand('linux', 'https://x.github.io/pdt/'), 'curl -fsSL https://x.github.io/pdt/install.sh | sh');
  assert.equal(installCommand('windows', 'https://pdt.dev/'), 'irm https://pdt.dev/install.ps1 | iex');
});

test('the visitor\'s system', () => {
  assert.equal(detectOs('Windows'), 'windows');
  assert.equal(detectOs('macOS'), 'mac');
  assert.equal(detectOs('Linux'), 'linux');
  assert.equal(detectOs(undefined, 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'), 'windows');
  assert.equal(detectOs(undefined, 'Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0'), 'linux');
  assert.equal(detectOs(undefined, 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'), 'mac');
  assert.equal(detectOs(undefined, 'Mozilla/5.0 (Linux; Android 14; Pixel 8)'), 'mac');
  assert.equal(detectOs(undefined, 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)'), 'mac');
  assert.equal(detectOs('', ''), 'mac');
});
