// @vitest-environment node
// Guard rails for the design system (spec 2026-10-17-design-system-ux §5.1): colours come from tokens.css, sizes and
// layers from the theme, confirmations from useConfirm, and features build on '@ui' rather than the libraries under it.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const web = dirname(dirname(fileURLToPath(new URL('./lint.test.ts', import.meta.url))));

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'gen' || name === 'node_modules' ? [] : files(path);
    return /\.(tsx?|css)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

const sources = [...files(join(web, 'src')), ...files(join(web, 'ui'))]
  .map((path) => ({ path: relative(web, path), text: readFileSync(path, 'utf8') }))
  .filter(({ path }) => path !== 'ui/tokens.css' && path !== 'ui/shadcn.css' && path !== 'src/lint.test.ts');

function offenders(pattern: RegExp, only: (path: string) => boolean = () => true) {
  return sources
    .filter(({ path }) => only(path))
    .flatMap(({ path, text }) =>
      text.split('\n').flatMap((line, i) => (pattern.test(line) ? [`${path}:${i + 1}: ${line.trim()}`] : [])),
    );
}

describe('design system rules', () => {
  it('defines colours only in tokens.css', () => {
    // Hex and rgb()/hsl() literals, and black/white utilities. CodeViewer and the terminal theme read tokens through
    // var(--pdt-*), which is fine.
    expect(offenders(/#[0-9a-fA-F]{3,8}\b(?![\w-])|\b(rgba?|hsla?)\(|\b(bg|text|border)-(black|white)\b/)).toEqual([]);
  });

  it('uses theme sizes and layers, not arbitrary ones', () => {
    expect(offenders(/\btext-\[\d+px\]|\bz-\[/)).toEqual([]);
    // App layers come from --pdt-z-*; a small z-10 inside a primitive only stacks that component's own parts.
    expect(offenders(/(?<![\w-])z-\d+\b/, (p) => !p.startsWith('ui/primitives/'))).toEqual([]);
  });

  it('confirms with useConfirm, never window.confirm', () => {
    expect(offenders(/\bwindow\.confirm\(|\bconfirm\(\s*['"`]/)).toEqual([]);
  });

  it('keeps React Aria, Base UI and the primitives behind @ui', () => {
    expect(
      offenders(/from ['"](react-aria-components|react-aria|react-stately|@base-ui\/react[^'"]*|@ui\/primitives[^'"]*)['"]/, (p) => !p.startsWith('ui/')),
    ).toEqual([]);
  });
});
