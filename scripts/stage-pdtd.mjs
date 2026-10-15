// Copies target/release/pdtd to desktop/src-tauri/binaries/pdtd-<host triple>, the name Tauri's externalBin expects.
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
copyFileSync(join(root, 'target', 'release', `pdtd${ext}`), join(out, `pdtd-${host}${ext}`));
console.log(`staged pdtd for ${host}`);
