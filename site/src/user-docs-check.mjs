// Keeps the user docs (docs/user/) free of references only the people building PDT understand: ticket keys, code paths,
// API routes, specs and plan phases. `pnpm --filter site test` runs it (so the `site` CI job does), and
// `pnpm --filter site check-docs` runs it alone. The READMEs in docs/user/ (notes for whoever writes the docs) are not
// published and not checked; nor are the agent-facing guides (agent-guide.md, prompts/), which live outside docs/user.
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const USER_DOCS = fileURLToPath(new URL('../../docs/user/', import.meta.url));

/** What counts as an internal reference, each with why it doesn't belong in a user doc. */
export const RULES = [
  { name: 'ticket key', pattern: /\bPDT-\d+\b/g, why: 'say what changed, not which ticket changed it' },
  { name: 'code path', pattern: /\b(crates|web\/src|web\/ui|desktop\/src-tauri|docs\/superpowers)\//g, why: 'readers use the app, not its source' },
  { name: 'source file', pattern: /\w\.(rs|tsx?)\b/g, why: 'readers use the app, not its source' },
  { name: 'crate', pattern: /\bpdt-(core|api|store|agents|git|forge|github|update|tailscale|int-[a-z]+)\b/g, why: 'name the part of PDT the reader sees' },
  { name: 'API route', pattern: /\/api\//g, why: 'describe the screen or the action, not the route behind it' },
  { name: 'spec', pattern: /\bspecs?\b/gi, why: 'specs are for the people building PDT' },
  { name: 'plan phase', pattern: /\bPhase \d+[a-z]?\b/g, why: 'plan phases are for the people building PDT' },
];

/**
 * Legitimate exceptions: `{ file, contains }`, where `file` is relative to docs/user/: a match on a line of that file
 * holding `contains` passes. Keep `contains` specific, so the exception covers that one use.
 */
export const ALLOW = [
  // GitHub Enterprise Server's API URL, which the reader types in.
  { file: 'getting-started/connect.md', contains: 'https://github.example.com/api/v3' },
  { file: 'your-machine/github.md', contains: 'https://github.example.com/api/v3' },
  // A tailnet address (`*.ts.net`), which the reader sees in Settings and opens on their phone.
  { file: 'your-machine/remote-access.md', contains: '.your-tailnet.ts.net' },
];

/** Every internal reference in `text` (the file at `file`, relative to docs/user/): `{ line, rule, text, why }`. */
export function findInternalRefs(text, file = '', allow = ALLOW) {
  const found = [];
  text.split('\n').forEach((line, i) => {
    for (const rule of RULES) {
      for (const m of line.matchAll(rule.pattern)) {
        if (allow.some((a) => a.file === file && line.includes(a.contains))) continue;
        found.push({ line: i + 1, rule: rule.name, text: m[0], why: rule.why });
      }
    }
  });
  return found;
}

/** The Markdown files under `dir` that get checked, relative to it, sorted. */
export function userDocFiles(dir = USER_DOCS) {
  return readdirSync(dir, { recursive: true })
    .map((f) => String(f).split(path.sep).join('/'))
    .filter((f) => f.endsWith('.md') && !/(^|\/)README\.md$/.test(f))
    .sort();
}

/** Every internal reference in the user docs, as `file:line: …` lines. */
export function checkUserDocs(dir = USER_DOCS) {
  return userDocFiles(dir).flatMap((f) =>
    findInternalRefs(readFileSync(path.join(dir, f), 'utf8'), f).map(
      (r) => `docs/user/${f}:${r.line}: ${r.rule} "${r.text}" (${r.why})`,
    ),
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const problems = checkUserDocs();
  if (problems.length) {
    console.error(`${problems.join('\n')}\n\n${problems.length} internal reference(s) in the user docs. Rewrite them for`);
    console.error('people who use PDT, or add a legitimate exception to ALLOW in site/src/user-docs-check.mjs.');
    process.exit(1);
  }
  console.log(`The user docs have no internal references (${userDocFiles().length} files).`);
}
