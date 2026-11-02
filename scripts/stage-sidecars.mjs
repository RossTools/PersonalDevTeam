// Copies target/release/{pdtd,pdt-host} to desktop/src-tauri/binaries/<name>-<host triple>, the names Tauri's
// externalBin expects. `--target <triple>` (the macOS release bundle builds each CPU on its own, `tauri build --target
// <triple>`) instead stages target/<triple>/release/<name> as <name>-<triple>.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
if (args.length && !(args.length === 2 && args[0] === '--target' && args[1])) {
  throw new Error('usage: node scripts/stage-sidecars.mjs [--target <triple>]');
}
const target = args[1];
const host = execFileSync('rustc', ['-vV'], { cwd: root, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]?.trim();
if (!host) throw new Error('could not read the host triple from `rustc -vV`');
const triple = target ?? host;
const ext = triple.includes('windows') ? '.exe' : '';
const from = target ? join(root, 'target', target, 'release') : join(root, 'target', 'release');
const out = join(root, 'desktop', 'src-tauri', 'binaries');
mkdirSync(out, { recursive: true });
for (const name of ['pdtd', 'pdt-host']) {
  copyFileSync(join(from, `${name}${ext}`), join(out, `${name}-${triple}${ext}`));
  console.log(`staged ${name} for ${triple}`);
}
