// @vitest-environment node
// The gallery shows every component (web/ui/README.md "Gallery"): each component '@ui' exports is rendered by name
// in a section under ./sections, or is listed below with where it shows instead.
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import * as ui from '@ui';

const sections = join(dirname(fileURLToPath(import.meta.url)), 'sections');
const source = readdirSync(sections)
  .filter((name) => name.endsWith('.tsx'))
  .map((name) => readFileSync(join(sections, name), 'utf8'))
  .join('\n');

/** Components the gallery shows through something else, each with what. */
const shownThrough: Record<string, string> = {
  MenuAt: 'useMenuAt, which renders it (Menus and overlays)',
};

it('renders every @ui component in a gallery section', () => {
  // Components are the PascalCase runtime exports (types aren't there at runtime; constants are UPPER_CASE).
  const components = Object.keys(ui).filter((name) => /^[A-Z][a-z]/.test(name));
  expect(components.length).toBeGreaterThan(50);
  const missing = components.filter((name) => !(name in shownThrough) && !new RegExp(`<${name}[\\s/>]`).test(source));
  expect(missing).toEqual([]);
  expect(Object.keys(shownThrough).filter((name) => !components.includes(name))).toEqual([]);
});
