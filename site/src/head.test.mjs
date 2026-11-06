import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { sharedHead } from './head.mjs';

const attr = (head, key, value) => head.find((t) => t.attrs[key] === value)?.attrs;

test('the preview image is an absolute URL under the base path', () => {
  const head = sharedHead('https://pdt.dev', '/');
  assert.equal(attr(head, 'property', 'og:image').content, 'https://pdt.dev/og.png');
  assert.equal(attr(head, 'name', 'twitter:image').content, 'https://pdt.dev/og.png');
  const sub = sharedHead('https://rosstools.github.io', '/PersonalDevTeam');
  assert.equal(attr(sub, 'property', 'og:image').content, 'https://rosstools.github.io/PersonalDevTeam/og.png');
  assert.equal(attr(sub, 'rel', 'apple-touch-icon').href, '/PersonalDevTeam/apple-touch-icon.png');
});

test('every file the tags name is in public/', () => {
  for (const t of sharedHead('https://pdt.dev', '/')) {
    const url = t.attrs.href ?? (t.attrs.content?.startsWith('https://') ? new URL(t.attrs.content).pathname : null);
    if (url) assert.ok(existsSync(new URL(`../public${url}`, import.meta.url)), url);
  }
});
