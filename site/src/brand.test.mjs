import { test } from 'node:test';
import assert from 'node:assert/strict';
import { NAME, SHORT_NAME, SITE_NAME, TAGLINE, pageTitle } from './brand.mjs';

test('the site name spells out what PDT stands for', () => {
  assert.equal(SHORT_NAME, 'PDT');
  assert.equal(NAME, 'Personal Dev Team');
  assert.equal(SITE_NAME, 'PDT (Personal Dev Team)');
});

test('the landing page title is the site name and tagline; other pages put theirs first', () => {
  assert.equal(pageTitle(), `PDT (Personal Dev Team): ${TAGLINE}`);
  assert.equal(TAGLINE, 'a dev team that lives on your machine');
  assert.equal(pageTitle('Features'), 'Features · PDT (Personal Dev Team)');
});
