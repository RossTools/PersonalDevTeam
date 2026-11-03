// The "PDT is built by PDT" numbers, counted from the repository's history at build time so they never go stale.
// `version` is the newest stable release tag (`vX.Y.Z`, no nightly or other pre-release) and `releases` counts the
// stable releases. A checkout without the history (a shallow clone) falls back to the last counted numbers.
import { execFileSync } from 'node:child_process';

const FALLBACK = { tickets: 211, prs: 206, releases: 158, version: null };

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}

/** The stable release tags (`vX.Y.Z`) among `tags`, newest first. */
export function stableTags(tags) {
  const parse = (t) => t.slice(1).split('.').map(Number);
  return tags
    .filter((t) => /^v\d+\.\d+\.\d+$/.test(t))
    .sort((a, b) => {
      const [x, y] = [parse(a), parse(b)];
      return y[0] - x[0] || y[1] - x[1] || y[2] - x[2];
    });
}

export function repoStats() {
  try {
    if (git('rev-parse', '--is-shallow-repository').trim() === 'true') return FALLBACK;
    const subjects = git('log', '--format=%s').split('\n');
    const keys = subjects.flatMap((s) => [...s.matchAll(/\bPDT-(\d+)\b/g)].map((m) => Number(m[1])));
    const prs = subjects.filter((s) => /\(#\d+\)$/.test(s)).length;
    const tags = stableTags(git('tag', '--list', 'v*').split('\n').filter(Boolean));
    return { tickets: Math.max(FALLBACK.tickets, ...keys), prs, releases: tags.length, version: tags[0] ?? null };
  } catch {
    return FALLBACK;
  }
}
