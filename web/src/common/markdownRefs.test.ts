import { describe, expect, it } from 'vitest';
import type { ForgeLinks } from '../api/gen/ForgeLinks';
import { fileHref, filePathOf, isSha, prNumberOf, prUrl, statusTone } from './markdownRefs';

const github: ForgeLinks = { kind: 'github', name: 'GitHub', repo: 'acme/shop', repoUrl: 'https://github.com/acme/shop', branchUrl: null, compareUrl: null };

describe('markdown refs', () => {
  it("builds and reads the forge's PR links", () => {
    expect(prUrl(github, 7)).toBe('https://github.com/acme/shop/pull/7');
    expect(prUrl({ ...github, kind: 'gitlab', repoUrl: 'https://gitlab.com/a/b/' }, 7)).toBe('https://gitlab.com/a/b/-/merge_requests/7');
    expect(prUrl({ ...github, kind: 'gitea' }, 7)).toBeNull();
    expect(prUrl(null, 7)).toBeNull();
    expect(prNumberOf(github, 'https://github.com/acme/shop/pull/12')).toBe(12);
    expect(prNumberOf(github, 'https://github.com/acme/shop/pull/12/files')).toBe(12);
    expect(prNumberOf(github, 'https://github.com/other/repo/pull/12')).toBeNull();
    expect(prNumberOf(github, 'https://github.com/acme/shop/pull/12abc')).toBeNull();
  });

  it('knows a commit SHA from a word or a number', () => {
    expect(isSha('4427552e')).toBe(true);
    expect(isSha('4427552e5f8a9c0d1e2f3a4b5c6d7e8f9a0b1c2d')).toBe(true);
    expect(isSha('deadbeef')).toBe(false);
    expect(isSha('1234567')).toBe(false);
    expect(isSha('abc12')).toBe(false);
  });

  it('reads a file path, without its line, and nothing else', () => {
    expect(filePathOf('src/checkout/session.ts')).toBe('src/checkout/session.ts');
    expect(filePathOf('./README.md')).toBe('README.md');
    expect(filePathOf('web/src/main.tsx:12')).toBe('web/src/main.tsx');
    expect(filePathOf('.github/workflows/ci.yml')).toBe('.github/workflows/ci.yml');
    for (const no of ['cargo test', 'https://x.test/a.md', '/etc/hosts', '~/x.md', '../up.md', 'v1.2.3', 'src/', 'PDT-12', 'useQuery()']) {
      expect(filePathOf(no), no).toBeNull();
    }
  });

  it('builds the file viewer path with the checkout', () => {
    const refs = { sessionId: 's', projectKey: 'SHOP', worktreeId: 'w1', forge: null };
    expect(fileHref(refs, 'src/a b.ts')).toBe('/files/src/a%20b.ts?project=SHOP&worktree=w1');
  });

  it('colours known status words only', () => {
    expect(statusTone('CI failed')).toBe('failed');
    expect(statusTone('Coder stalled')).toBe('failed');
    expect(statusTone('CI passed')).toBe('running');
    expect(statusTone('Coder working')).toBe('running');
    expect(statusTone('in review')).toBe('waiting');
    expect(statusTone('Ready to accept')).toBe('waiting');
    expect(statusTone('blocked by SHOP-31')).toBe('neutral');
    expect(statusTone('Guest checkout')).toBeNull();
    expect(statusTone('The run failed because the sandbox has no Chrome')).toBeNull();
  });
});
