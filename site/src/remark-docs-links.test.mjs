import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rewriteLink } from './remark-docs-links.mjs';
import { docId, isPublished } from './docs-pages.mjs';

const REPO = 'https://github.com/RossTools/PersonalDevTeam/blob/main';

test('links between published docs go to their pages', () => {
  assert.equal(rewriteLink('tickets.md', 'reference/agents.md'), '/docs/reference/tickets/');
  assert.equal(rewriteLink('workflows.md#inbox', 'reference/agents.md'), '/docs/reference/workflows/#inbox');
  assert.equal(rewriteLink('../SDLC.md', 'reference/host.md'), '/docs/sdlc/');
  assert.equal(rewriteLink('reference/host.md', 'SDLC.md'), '/docs/reference/host/');
});

test('the base path is kept', () => {
  assert.equal(rewriteLink('tickets.md', 'reference/agents.md', '/pdt'), '/pdt/docs/reference/tickets/');
});

test('links to unpublished files go to GitHub', () => {
  assert.equal(rewriteLink('ROADMAP.md', 'SDLC.md'), `${REPO}/docs/ROADMAP.md`);
  assert.equal(rewriteLink('superpowers/notes/ci.md', 'SDLC.md'), `${REPO}/docs/superpowers/notes/ci.md`);
  assert.equal(rewriteLink('../../AGENTS.md#rules', 'reference/host.md'), `${REPO}/AGENTS.md#rules`);
});

test('absolute links and anchors are left alone', () => {
  for (const href of ['https://example.com/a.md', '#notifications', '/api/guides/agents', 'mailto:x@y.z']) {
    assert.equal(rewriteLink(href, 'reference/agents.md'), href);
  }
});

test('which docs are published', () => {
  assert.ok(isPublished('SDLC.md'));
  assert.ok(isPublished('reference/tickets.md'));
  assert.ok(!isPublished('ROADMAP.md'));
  assert.ok(!isPublished('prompts/coder.md'));
  assert.ok(!isPublished('superpowers/specs/x.md'));
  assert.equal(docId('SDLC.md'), 'docs/sdlc');
});
