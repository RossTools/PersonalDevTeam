import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { checkUserDocs, findInternalRefs, userDocFiles, USER_DOCS } from './user-docs-check.mjs';
import { USER_SECTIONS } from './docs-pages.mjs';

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
