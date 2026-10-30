import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CATEGORIES, PLUGINS, groupPlugins, inlineCode, pluginsOn } from './plugins.mjs';

test('the media plugins are on their own page, every other one on the main page', () => {
  const main = pluginsOn(PLUGINS, false);
  const media = pluginsOn(PLUGINS, true);
  assert.ok(main.length > 0 && media.length > 0);
  assert.ok(main.every((p) => p.category !== 'media'));
  assert.ok(media.every((p) => p.category === 'media'));
  assert.equal(main.length + media.length, PLUGINS.length);
  assert.deepEqual(media.map((p) => p.id).sort(), ['arr', 'plex', 'scryer']);
});

test('every plugin has a unique id, a known category and what its card shows', () => {
  assert.equal(new Set(PLUGINS.map((p) => p.id)).size, PLUGINS.length);
  for (const p of PLUGINS) {
    assert.ok(CATEGORIES.includes(p.category), p.id);
    assert.match(p.id, /^[a-z0-9-]+$/);
    assert.ok(p.label && p.description && p.icon.letters && p.icon.color, p.id);
  }
});

test('groups follow the category order and leave out empty ones', () => {
  assert.deepEqual(groupPlugins(pluginsOn(PLUGINS, false)).map(([c]) => c), ['dev', 'chat', 'general']);
});

test('inline code in descriptions, the rest escaped', () => {
  assert.equal(inlineCode('leaves by `exists` <b>'), 'leaves by <code>exists</code> &lt;b&gt;');
});
