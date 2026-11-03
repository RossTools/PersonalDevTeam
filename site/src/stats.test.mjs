import { test } from 'node:test';
import assert from 'node:assert/strict';
import { stableTags } from './stats.mjs';

test('the version is the newest stable tag, never a nightly', () => {
  const tags = ['v0.6.1-nightly.20261103', 'v0.6.0', 'v0.5.1-nightly.20261102.2', 'v0.5.1-rc.1', 'v0.5.0', 'v0.10.0-beta', 'vnext'];
  assert.deepEqual(stableTags(tags), ['v0.6.0', 'v0.5.0']);
});

test('stable tags in version order, newest first', () => {
  assert.deepEqual(stableTags(['v0.9.0', 'v0.10.0', 'v1.0.0', 'v0.10.1']), ['v1.0.0', 'v0.10.1', 'v0.10.0', 'v0.9.0']);
  assert.deepEqual(stableTags(['v0.1.0-nightly.1']), []);
});
