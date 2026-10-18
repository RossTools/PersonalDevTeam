// Screenshots of the component gallery for PRs that touch web/ui (spec D4: for review, not a CI gate).
//
//   pnpm --dir web dev                                  # serves /gallery.html (in another terminal)
//   pnpm --dir web exec playwright-core install chromium   # once
//   node web/scripts/ui-screenshots.mjs <out dir> [base url]
//
// Writes gallery-{light,dark}-{desktop,phone}.png (full page) into <out dir>.
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { chromium } from 'playwright-core';

const [outArg, base = 'http://localhost:5173'] = process.argv.slice(2);
if (!outArg) {
  console.error('usage: node web/scripts/ui-screenshots.mjs <out dir> [base url]');
  process.exit(2);
}
const out = resolve(outArg);
mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
try {
  for (const theme of ['light', 'dark']) {
    for (const [name, viewport] of [['desktop', { width: 1440, height: 900 }], ['phone', { width: 390, height: 844 }]]) {
      const page = await browser.newPage({ viewport, colorScheme: theme, hasTouch: name === 'phone' });
      // The gallery reads the same theme preference as the app.
      await page.addInitScript((t) => localStorage.setItem('pdt.theme', t), theme);
      await page.goto(`${base}/gallery.html${name === 'phone' ? '?phone' : ''}`);
      await page.waitForLoadState('networkidle');
      const file = join(out, `gallery-${theme}-${name}.png`);
      await page.screenshot({ path: file, fullPage: true });
      console.log(file);
      await page.close();
    }
  }
} finally {
  await browser.close();
}
