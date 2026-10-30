import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rewriteLink } from './remark-docs-links.mjs';
import { docId, isPublished } from './docs-pages.mjs';

const REPO = 'https://github.com/RossTools/PersonalDevTeam/blob/main';

test('links between published docs go to their pages', () => {
  assert.equal(rewriteLink('testing.md', 'dev/architecture.md'), '/docs/dev/testing/');
  assert.equal(rewriteLink('invariants.md#agents', 'dev/architecture.md'), '/docs/dev/invariants/#agents');
  assert.equal(rewriteLink('../SDLC.md', 'dev/architecture.md'), '/docs/sdlc/');
  assert.equal(rewriteLink('dev/architecture.md', 'SDLC.md'), '/docs/dev/architecture/');
});

test('the user docs are the main docs, without user/ in their paths', () => {
  assert.equal(rewriteLink('install.md', 'user/getting-started/first-run.md'), '/docs/getting-started/install/');
  assert.equal(rewriteLink('../automation/the-pm.md#ask', 'user/getting-started/next-steps.md'), '/docs/automation/the-pm/#ask');
  assert.equal(rewriteLink('README.md', 'user/getting-started/install.md'), `${REPO}/docs/user/getting-started/README.md`);
  assert.equal(rewriteLink('../README.md', 'user/getting-started/install.md'), `${REPO}/docs/user/README.md`);
});

test('the base path is kept', () => {
  assert.equal(rewriteLink('testing.md', 'dev/architecture.md', '/pdt'), '/pdt/docs/dev/testing/');
});

test('links to unpublished files go to GitHub', () => {
  assert.equal(rewriteLink('ROADMAP.md', 'SDLC.md'), `${REPO}/docs/ROADMAP.md`);
  assert.equal(rewriteLink('superpowers/notes/ci.md', 'SDLC.md'), `${REPO}/docs/superpowers/notes/ci.md`);
  assert.equal(rewriteLink('../../AGENTS.md#rules', 'dev/architecture.md'), `${REPO}/AGENTS.md#rules`);
});

test('absolute links and anchors are left alone', () => {
  for (const href of ['https://example.com/a.md', '#notifications', '/api/guides/agents', 'mailto:x@y.z']) {
    assert.equal(rewriteLink(href, 'dev/architecture.md'), href);
  }
});

test('which docs are published', () => {
  assert.ok(isPublished('SDLC.md'));
  assert.ok(isPublished('dev/architecture.md'));
  assert.ok(!isPublished('reference/tickets.md'));
  assert.ok(!isPublished('ROADMAP.md'));
  assert.ok(!isPublished('prompts/coder.md'));
  assert.ok(!isPublished('superpowers/specs/x.md'));
  assert.ok(isPublished('user/getting-started/install.md'));
  assert.ok(!isPublished('user/README.md'));
  assert.equal(docId('SDLC.md'), 'docs/sdlc');
  assert.equal(docId('user/getting-started/install.md'), 'docs/getting-started/install');
});
