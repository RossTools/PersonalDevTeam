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

/** WCAG contrast ratio of two `#rrggbb` colours. */
function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

describe('light contrast (PDT-249)', () => {
  const light = block(':root {');
  const get = (t: string) => {
    const v = light.get(t);
    expect(v, t).toMatch(/^#[0-9a-f]{6}$/i);
    return v!;
  };

  it.each(['--pdt-bg', '--pdt-surface', '--pdt-surface-2'])('muted text passes AA (4.5:1) on %s', (bg) => {
    expect(contrast(get('--pdt-muted'), get(bg))).toBeGreaterThanOrEqual(4.5);
  });

  it('control edges stand out from the surface (2.3:1; the softer setting, below 3:1 on purpose)', () => {
    expect(contrast(get('--pdt-input'), get('--pdt-surface'))).toBeGreaterThanOrEqual(2.3);
  });
});

describe('dark mode keeps its look (PDT-249)', () => {
  const dark = block(":root[data-theme='dark']");

  it('draws control edges in the border colour and cards without a shadow', () => {
    expect(dark.get('--pdt-input')).toBe(dark.get('--pdt-border'));
    expect(dark.get('--pdt-shadow-card')).toBe('none');
  });
});
