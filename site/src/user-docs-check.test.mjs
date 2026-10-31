import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { checkUserDocs, designDocs, findInternalRefs, userDocFiles, USER_DOCS } from './user-docs-check.mjs';
import { DEVELOPER_PAGES, USER_SECTIONS } from './docs-pages.mjs';

const DOCS = path.join(USER_DOCS, '..');

const hits = (text, file, allow) => findInternalRefs(text, file, allow).map((r) => `${r.rule}:${r.text}`);

test('finds internal references', () => {
  assert.deepEqual(hits('Fixed in PDT-230.'), ['ticket key:PDT-230']);
  assert.deepEqual(hits('See crates/pdt-api/src/start.rs'), ['code path:crates/', 'source file:t.rs', 'crate:pdt-api']);
  assert.deepEqual(hits('web/src/setup/SetupPage.tsx'), ['code path:web/src/', 'source file:e.tsx']);
  assert.deepEqual(hits('`GET /api/host/setup`'), ['API route:/api/']);
  assert.deepEqual(hits('The Spec says so, built in Phase 7c.'), ['spec:Spec', 'plan phase:Phase 7c']);
});

test('leaves ordinary prose alone', () => {
  const prose = [
    'A ticket key looks like ALP-12. Run pdt-host install, then open http://127.0.0.1:7420.',
    'Pick a specific agent; the next phase of the work starts on its own.',
    'Copy the .app into Applications. Node.js and the PDT UI.',
  ];
  for (const line of prose) assert.deepEqual(hits(line), [], line);
});

test('reports the line', () => {
  assert.equal(findInternalRefs('one\ntwo PDT-1\n')[0].line, 2);
});

test('an allowed line passes, only in its own file', () => {
  const allow = [{ file: 'a/b.md', contains: '127.0.0.1:7420/api/health' }];
  assert.deepEqual(hits('curl 127.0.0.1:7420/api/health', 'a/b.md', allow), []);
  assert.deepEqual(hits('curl 127.0.0.1:7420/api/health', 'a/c.md', allow), ['API route:/api/']);
  assert.deepEqual(hits('see /api/tickets', 'a/b.md', allow), ['API route:/api/']);
});

test('the user docs have no internal references', () => {
  assert.deepEqual(checkUserDocs(), []);
});

test('every page in the sidebar exists, and every page is in the sidebar', () => {
  const listed = USER_SECTIONS.flatMap((s) => s.pages.map((p) => `${s.dir}/${p}.md`)).sort();
  for (const f of listed) assert.ok(existsSync(path.join(USER_DOCS, f)), `missing docs/user/${f}`);
  assert.deepEqual(userDocFiles(), listed);
});

test('the developer guide is in the sidebar, and every page of it is', () => {
  for (const f of DEVELOPER_PAGES) assert.ok(existsSync(path.join(DOCS, f)), `missing docs/${f}`);
  const dev = readdirSync(path.join(DOCS, 'dev')).filter((f) => f.endsWith('.md')).map((f) => `dev/${f}`);
  assert.deepEqual(dev.sort(), DEVELOPER_PAGES.filter((f) => f.startsWith('dev/')).sort());
});

test('the developer guide and the guides pdtd serves name no tickets', () => {
  const prompts = readdirSync(path.join(DOCS, 'prompts')).map((f) => `prompts/${f}`);
  const files = [...DEVELOPER_PAGES, 'agent-guide.md', 'run-config.md', ...prompts];
  const found = files.flatMap((f) =>
    readFileSync(path.join(DOCS, f), 'utf8')
      .split('\n')
      .flatMap((line, i) => (line.match(/\bPDT-\d+\b/g) ?? []).map((k) => `docs/${f}:${i + 1}: ${k}`)),
  );
  assert.deepEqual(found, []);
});

test('docs/ holds no design specs or plans', () => {
  assert.deepEqual(designDocs(), []);
});

test('finds design specs and plans anywhere under docs/', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'pdt-docs-'));
  try {
    for (const f of ['superpowers/notes/x.md', 'dev/specs/a.md', 'plans/b.md', 'user/x/explains-plans.md', 'dev/c.md']) {
      mkdirSync(path.dirname(path.join(dir, f)), { recursive: true });
      writeFileSync(path.join(dir, f), '# x\n');
    }
    assert.deepEqual(designDocs(dir), ['dev/specs/a.md', 'plans/b.md', 'superpowers/notes/x.md']);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
