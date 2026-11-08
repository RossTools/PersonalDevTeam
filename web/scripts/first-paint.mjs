// The JavaScript the first paint loads: dist/index.html's entry and the chunks it preloads, gzipped. Run after
// `vite build` (pnpm build does); it fails when that grows past the budget, so growth is seen in review. A screen most
// visits never open goes behind lazy() (src/shell/screens.tsx) rather than raising the budget.
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

const BUDGET_KB = 400;

const dist = new URL('../dist/', import.meta.url);
const html = readFileSync(new URL('index.html', dist), 'utf8');
const files = [...new Set([...html.matchAll(/assets\/[^"]+\.js/g)].map((m) => m[0]))];
let raw = 0;
let gzip = 0;
for (const f of files) {
  const bytes = readFileSync(new URL(f, dist));
  raw += bytes.length;
  gzip += gzipSync(bytes).length;
}
const kb = (n) => Math.round(n / 1000);
console.log(`first paint: ${files.length} scripts, ${kb(raw)} kB, ${kb(gzip)} kB gzip (budget ${BUDGET_KB} kB gzip)`);
if (gzip > BUDGET_KB * 1000) {
  console.error(`first paint is over its budget: lazy-load what it doesn't need (src/shell/screens.tsx), or raise BUDGET_KB in web/scripts/first-paint.mjs`);
  process.exit(1);
}
