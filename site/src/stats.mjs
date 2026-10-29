// The "PDT is built by PDT" numbers, counted from the repository's history at build time so they never go stale.
// `version` is the newest release tag. A checkout without the history (a shallow clone) falls back to the last counted numbers.
import { execFileSync } from 'node:child_process';

const FALLBACK = { tickets: 211, prs: 206, releases: 153, version: null };

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}

export function repoStats() {
  try {
    if (git('rev-parse', '--is-shallow-repository').trim() === 'true') return FALLBACK;
    const subjects = git('log', '--format=%s').split('\n');
    const keys = subjects.flatMap((s) => [...s.matchAll(/\bPDT-(\d+)\b/g)].map((m) => Number(m[1])));
    const prs = subjects.filter((s) => /\(#\d+\)$/.test(s)).length;
    const tags = git('tag', '--list', 'v*', '--sort=-v:refname').split('\n').filter(Boolean);
    return { tickets: Math.max(FALLBACK.tickets, ...keys), prs, releases: tags.length, version: tags[0] ?? null };
  } catch {
    return FALLBACK;
  }
}
