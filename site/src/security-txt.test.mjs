import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { securityTxt, VALID_DAYS } from './security-txt.mjs';

const field = (txt, name) => txt.split('\n').filter((l) => l.startsWith(`${name}: `)).map((l) => l.slice(name.length + 2));

test('security.txt names the email and the private reporting form, email first', () => {
  const txt = securityTxt('https://pdt.dev/.well-known/security.txt');
  assert.deepEqual(field(txt, 'Contact'), [
    'mailto:security@pdt.dev',
    'https://github.com/RossTools/PersonalDevTeam/security/advisories/new',
  ]);
  assert.deepEqual(field(txt, 'Canonical'), ['https://pdt.dev/.well-known/security.txt']);
  assert.deepEqual(field(txt, 'Policy'), ['https://github.com/RossTools/PersonalDevTeam/blob/main/SECURITY.md']);
});

test('it expires less than a year after the build, in RFC 3339 UTC', () => {
  const now = new Date('2026-11-03T10:20:30.456Z');
  const [expires] = field(securityTxt('https://pdt.dev/.well-known/security.txt', now), 'Expires');
  assert.equal(expires, '2027-05-02T10:20:30Z');
  const days = (Date.parse(expires) - now.getTime()) / 86_400_000;
  assert.ok(days > 30 && days < 365, `${days} days`);
  assert.ok(VALID_DAYS < 365);
});

test('it matches the contacts SECURITY.md gives', () => {
  const policy = readFileSync(new URL('../../SECURITY.md', import.meta.url), 'utf8');
  for (const contact of field(securityTxt('https://pdt.dev/.well-known/security.txt'), 'Contact')) {
    assert.ok(policy.includes(contact.replace('mailto:', '')), `${contact} is in SECURITY.md`);
  }
});
