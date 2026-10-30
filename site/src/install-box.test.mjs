// The install box's classes are all its own (`install…`): a visitor's browser extension can style a generic class such
// as `.default` (one painted the macOS panel blue, PDT-298), so the box never uses one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const box = readFileSync(new URL('./components/InstallBox.astro', import.meta.url), 'utf8');
const css = readFileSync(new URL('./styles/landing.css', import.meta.url), 'utf8');

test('the install box uses only install… classes', () => {
	const classes = new Set();
	for (const [, list] of box.matchAll(/class=\{?[^'"]*['"]([^'"]+)['"]/g)) list.split(/\s+/).forEach((c) => classes.add(c));
	for (const [, list] of box.matchAll(/class=\{i === 0 \? '([^']+)' : '([^']+)'\}/g)) list.split(/\s+/).forEach((c) => classes.add(c));
	for (const [, c] of box.matchAll(/box\.classList\.\w+\('([^']+)'/g)) classes.add(c);
	assert.ok(classes.has('install-default') && classes.has('install-ready'), [...classes].join(' '));
	for (const c of classes) assert.match(c, /^install(-|$)/, `class "${c}"`);
});

test('the install box styles name only install… classes', () => {
	const start = css.indexOf('/* The install command');
	const end = css.indexOf('/* Final CTA */');
	const rules = css.slice(start, end).replace(/\/\*[\s\S]*?\*\//g, '');
	for (const [, c] of rules.matchAll(/\.([a-z][\w-]*)/g)) assert.match(c, /^install(-|$)/, `class ".${c}"`);
});
