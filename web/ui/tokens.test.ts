import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');
const tokens = read('./tokens.css');

/** `--pdt-*` declarations in the first `{…}` block after `marker`. */
function block(marker: string): Map<string, string> {
  const start = tokens.indexOf(marker);
  expect(start, marker).toBeGreaterThanOrEqual(0);
  // The media query nests one more level; take the innermost block that holds declarations.
  let open = tokens.indexOf('{', start);
  const nested = tokens.indexOf('{', open + 1);
  const close = tokens.indexOf('}', open);
  if (nested !== -1 && nested < close) open = nested;
  const body = tokens.slice(open + 1, tokens.indexOf('}', open));
  return new Map([...body.matchAll(/(--pdt-[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

describe('tokens.css', () => {
  it('writes the dark palette the same way in both places', () => {
    const system = block('@media (prefers-color-scheme: dark)');
    const picked = block(":root[data-theme='dark']");
    expect(system.size).toBeGreaterThan(20);
    expect(Object.fromEntries(picked)).toEqual(Object.fromEntries(system));
  });

  it('gives every themed light token a dark value', () => {
    const light = block(':root {');
    const dark = block(":root[data-theme='dark']");
    const themeless = /^--pdt-(z-|control-|duration|ease|radius|font-)/;
    const missing = [...light.keys()].filter((k) => !themeless.test(k) && !dark.has(k));
    expect(missing).toEqual([]);
  });

  it('defines every token index.css uses', () => {
    const used = new Set([...read('../src/index.css').matchAll(/var\((--pdt-[\w-]+)\)/g)].map((m) => m[1]));
    const defined = block(':root {');
    expect([...used].filter((t) => !defined.has(t))).toEqual([]);
  });
});
