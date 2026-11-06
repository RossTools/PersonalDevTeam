import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { docDescription } from './doc-description.mjs';

test("a page's description is its first paragraph, as plain text", () => {
  const md = '# Secrets\n\nGive **agents** the [keys](../a.md) and `tokens`\nthey *need*.\n\nMore.\n';
  assert.equal(docDescription(md), 'Give agents the keys and tokens they need.');
});

test('headings, lists, code and quotes before it are skipped', () => {
  const md = '# T\n\n> A note.\n\n```sh\necho hi\n```\n\n- a list\n\nThe paragraph.\n- then a list\n';
  assert.equal(docDescription(md), 'The paragraph.');
});

test('a long paragraph ends at a sentence that fits', () => {
  const first = 'PDT is a dev team that lives on your machine, with agents that build what you write in tickets.';
  const md = `# T\n\n${first} Then ${'a long second sentence that goes on '.repeat(3)}and on.\n`;
  assert.equal(docDescription(md), first);
});

test('without a sentence that fits, it ends at a word, with an ellipsis', () => {
  const md = `# T\n\n${'word '.repeat(60)}\n`;
  const d = docDescription(md);
  assert.ok(d.length <= 161 && d.endsWith('word…'), d);
});

test('every published docs page has a description of its own', async () => {
  const { DEVELOPER_PAGES, USER_SECTIONS } = await import('./docs-pages.mjs');
  const files = [
    ...USER_SECTIONS.flatMap((s) => s.pages.map((p) => `user/${s.dir}/${p}.md`)),
    ...DEVELOPER_PAGES,
    'agent-guide.md',
  ];
  const seen = new Map();
  for (const f of files) {
    const d = docDescription(readFileSync(new URL(`../../docs/${f}`, import.meta.url), 'utf8'));
    assert.ok(d.length >= 40, `${f}: "${d}"`);
    assert.ok(!seen.has(d), `${f} and ${seen.get(d)} share "${d}"`);
    seen.set(d, f);
  }
});
