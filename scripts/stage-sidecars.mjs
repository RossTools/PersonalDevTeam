// Copies target/release/{pdtd,pdt-host} to desktop/src-tauri/binaries/<name>-<host triple>, the names Tauri's
// externalBin expects.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const host = execFileSync('rustc', ['-vV'], { cwd: root, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]?.trim();
if (!host) throw new Error('could not read the host triple from `rustc -vV`');
const ext = process.platform === 'win32' ? '.exe' : '';
const out = join(root, 'desktop', 'src-tauri', 'binaries');
mkdirSync(out, { recursive: true });
for (const name of ['pdtd', 'pdt-host']) {
  copyFileSync(join(root, 'target', 'release', `${name}${ext}`), join(out, `${name}-${host}${ext}`));
  console.log(`staged ${name} for ${host}`);
}
