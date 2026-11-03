// @vitest-environment node
// cn must know every type size and spacing name src/index.css defines, or tailwind-merge misreads them.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { cn } from './utils';

const css = readFileSync(fileURLToPath(new URL('../../src/index.css', import.meta.url)), 'utf8');

describe('cn and the theme', () => {
  it('knows every type size and spacing name the theme defines', () => {
    // A theme name cn doesn't know is read as a colour (or kept beside the class it replaces): cn('text-x', 'text-fg')
    // would drop the size.
    const names = (prefix: string) => [...css.matchAll(new RegExp(`--${prefix}-([\\w-]+?):`, 'g'))]
      .map((m) => m[1]).filter((n) => !n.includes('--'));
    expect(names('text')).toContain('heading');
    expect(names('spacing')).toContain('gutter');
    for (const n of names('text')) expect(cn(`text-${n}`, 'text-muted-foreground'), n).toContain(`text-${n}`);
    for (const n of names('spacing')) expect(cn('px-3', `px-${n}`), n).toBe(`px-${n}`);
  });
});
