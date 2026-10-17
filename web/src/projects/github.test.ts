import { describe, expect, it } from 'vitest';
import { branchUrl, compareUrl, repoUrl } from './github';

describe('GitHub links', () => {
  it('builds repo, branch and compare links with encoded branch names', () => {
    expect(repoUrl('o/r')).toBe('https://github.com/o/r');
    expect(branchUrl('o/r', 'feature/x y')).toBe('https://github.com/o/r/tree/feature/x%20y');
    expect(compareUrl('o/r', 'main', 'feature/x')).toBe('https://github.com/o/r/compare/main...feature/x?expand=1');
  });
});
