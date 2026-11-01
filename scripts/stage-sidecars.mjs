// Copies target/release/{pdtd,pdt-host} to desktop/src-tauri/binaries/<name>-<host triple>, the names Tauri's
// externalBin expects. `--target universal-apple-darwin` (the macOS release bundle, `tauri build --target
// universal-apple-darwin`) instead stages target/{aarch64,x86_64}-apple-darwin/release/<name> under their triples and
// joined with lipo as <name>-universal-apple-darwin.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const UNIVERSAL = 'universal-apple-darwin';
const UNIVERSAL_PARTS = ['aarch64-apple-darwin', 'x86_64-apple-darwin'];

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const universal = args.length === 2 && args[0] === '--target' && args[1] === UNIVERSAL;
if (args.length && !universal) {
  throw new Error(`usage: node scripts/stage-sidecars.mjs [--target ${UNIVERSAL}]`);
}
const host = execFileSync('rustc', ['-vV'], { cwd: root, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]?.trim();
if (!host) throw new Error('could not read the host triple from `rustc -vV`');
const ext = process.platform === 'win32' ? '.exe' : '';
const out = join(root, 'desktop', 'src-tauri', 'binaries');
mkdirSync(out, { recursive: true });
for (const name of ['pdtd', 'pdt-host']) {
  if (universal) {
    // Tauri builds the app once per CPU before joining them, and each of those builds wants its own sidecar too.
    const parts = UNIVERSAL_PARTS.map((t) => join(root, 'target', t, 'release', name));
    UNIVERSAL_PARTS.forEach((t, i) => copyFileSync(parts[i], join(out, `${name}-${t}`)));
    execFileSync('lipo', ['-create', '-output', join(out, `${name}-${UNIVERSAL}`), ...parts], { stdio: 'inherit' });
    console.log(`staged ${name} for ${[...UNIVERSAL_PARTS, UNIVERSAL].join(', ')}`);
  } else {
    copyFileSync(join(root, 'target', 'release', `${name}${ext}`), join(out, `${name}-${host}${ext}`));
    console.log(`staged ${name} for ${host}`);
  }
}
