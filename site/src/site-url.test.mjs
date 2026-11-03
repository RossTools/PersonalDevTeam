import { test } from 'node:test';
import assert from 'node:assert/strict';
import { siteUrl } from './site-url.mjs';

test('the Pages origin is always https', () => {
  assert.equal(siteUrl({ SITE_URL: 'http://pdt.dev' }), 'https://pdt.dev');
  assert.equal(siteUrl({ SITE_URL: 'https://pdt.dev' }), 'https://pdt.dev');
  assert.equal(siteUrl({ SITE_URL: 'http://x.github.io' }), 'https://x.github.io');
});

test('locally the site stays on http://localhost', () => {
  assert.equal(siteUrl({}), 'http://localhost:4321');
  assert.equal(siteUrl({ SITE_URL: '' }), 'http://localhost:4321');
  assert.equal(siteUrl({ SITE_URL: 'http://localhost:4000' }), 'http://localhost:4000');
  assert.equal(siteUrl({ SITE_URL: 'http://127.0.0.1:4321' }), 'http://127.0.0.1:4321');
});
