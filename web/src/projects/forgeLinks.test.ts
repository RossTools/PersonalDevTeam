import { describe, expect, it } from 'vitest';
import type { ForgeLinks } from '../api/gen/ForgeLinks';
import { branchUrl, compareUrl, repoUrl } from './forgeLinks';

const forge: ForgeLinks = {
  kind: 'gitea',
  name: 'Gitea',
  repo: 'o/r',
  repoUrl: 'https://git.example.org/o/r',
  branchUrl: 'https://git.example.org/o/r/src/branch/{branch}',
  compareUrl: 'https://git.example.org/o/r/compare/{base}...{branch}',
};

describe('forge links', () => {
  it('fills the forge’s pages with encoded branch names', () => {
    expect(repoUrl(forge)).toBe('https://git.example.org/o/r');
    expect(branchUrl(forge, 'feature/x y')).toBe('https://git.example.org/o/r/src/branch/feature/x%20y');
    expect(branchUrl(forge, 'a$&b')).toBe('https://git.example.org/o/r/src/branch/a%24%26b');
    expect(compareUrl(forge, 'main', 'feature/x')).toBe('https://git.example.org/o/r/compare/main...feature/x');
  });

  it('has no branch or compare link when the forge has no such page', () => {
    const bare = { ...forge, branchUrl: null, compareUrl: null };
    expect(branchUrl(bare, 'x')).toBeNull();
    expect(compareUrl(bare, 'main', 'x')).toBeNull();
  });
});
