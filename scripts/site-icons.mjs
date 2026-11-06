// The website's link preview image and the icons browsers and phones ask for, made from the logo set and written to
// site/public/: og.png (1200×630, the picture a shared link shows), favicon.ico (16 and 32 px) and
// apple-touch-icon.png (180 px, square, as iOS rounds it itself). Run it from the checkout when the logo, the tagline
// or the hero's wording changes:
//
//   pnpm --dir web exec playwright-core install chromium   # once
//   node scripts/site-icons.mjs [--out <dir>]
//
// The preview is drawn by Chromium with the site's own Inter (so it looks the same on any machine), the icons by sharp
// from public/pdt-icon.svg.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// playwright-core is the web package's, sharp and the fonts the site's.
const { chromium } = createRequire(join(root, 'web/package.json'))('playwright-core');
const siteRequire = createRequire(join(root, 'site/package.json'));
const sharp = siteRequire('sharp');

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const out = resolve(outIdx >= 0 ? args[outIdx + 1] : join(root, 'site/public'));

const icon = readFileSync(join(root, 'site/public/pdt-icon.svg'));
const mark = readFileSync(join(root, 'site/src/assets/pdt-mark-on-dark.svg'), 'utf8');
// Inline, as a page set from a string can't load file: URLs.
const font = (weight) =>
  `data:font/woff2;base64,${readFileSync(siteRequire.resolve(`@fontsource/inter/files/inter-latin-${weight}-normal.woff2`)).toString('base64')}`;

// The landing page's hero, at link preview size: the mark, its headline and subtitle, in the site's colours.
const html = `<!doctype html>
<style>
  @font-face { font-family: Inter; font-weight: 400; src: url(${font(400)}); }
  @font-face { font-family: Inter; font-weight: 700; src: url(${font(700)}); }
  html, body { margin: 0; }
  body {
    width: 1200px; height: 630px; box-sizing: border-box; padding: 72px 88px;
    display: flex; flex-direction: column; justify-content: space-between;
    background: #0e0f12; color: #e7e8ec; font-family: Inter, sans-serif;
    background-image: radial-gradient(circle, rgba(53,97,184,.22) 1.5px, transparent 1.6px);
    background-size: 28px 28px;
  }
  .mark svg { height: 64px; width: auto; display: block; }
  h1 { font-size: 76px; line-height: 1.05; letter-spacing: -.03em; font-weight: 700; margin: 0; max-width: 940px; }
  h1 em { font-style: normal; color: #5b86d9; }
  p { font-size: 28px; line-height: 1.4; color: #8a8f9c; margin: 24px 0 0; max-width: 900px; }
  .foot { font-size: 24px; color: #8a8f9c; }
  .foot b { color: #e7e8ec; font-weight: 700; }
</style>
<div class="mark">${mark}</div>
<div>
  <h1>A dev team that lives <em>on your machine.</em></h1>
  <p>Agents build your tickets in their own worktrees, review and test them. You approve what ships.</p>
</div>
<div class="foot"><b>PDT</b> · Personal Dev Team · Open source · pdt.dev</div>`;

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
  await page.setContent(html);
  await page.evaluate(() => document.fonts.ready);
  if (!(await page.evaluate(() => document.fonts.check('700 76px Inter')))) throw new Error('Inter did not load');
  const shot = await page.screenshot({ type: 'png' });
  writeFileSync(join(out, 'og.png'), await sharp(shot).png({ palette: true, quality: 90 }).toBuffer());
} finally {
  await browser.close();
}

// iOS draws its own rounded corners: the icon's corners are filled with its background rather than left clear.
const png = (size, background) =>
  sharp(icon, { density: 72 * (size / 64) * 2 })
    .resize(size, size)
    .flatten(background ? { background } : false)
    .png()
    .toBuffer();
writeFileSync(join(out, 'apple-touch-icon.png'), await png(180, '#0e0f12'));
writeFileSync(join(out, 'favicon.ico'), ico([await png(16), await png(32)]));

/** An ICO holding PNG images (every browser since IE 11 reads them). */
function ico(images) {
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  images.forEach((img, i) => {
    const size = img.readUInt32BE(16); // the PNG's IHDR width
    const at = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, at);
    header.writeUInt8(size >= 256 ? 0 : size, at + 1);
    header.writeUInt16LE(1, at + 4);
    header.writeUInt16LE(32, at + 6);
    header.writeUInt32LE(img.length, at + 8);
    header.writeUInt32LE(offset, at + 12);
    offset += img.length;
  });
  return Buffer.concat([header, ...images]);
}

console.log(`site-icons: wrote og.png, apple-touch-icon.png and favicon.ico to ${out}`);
