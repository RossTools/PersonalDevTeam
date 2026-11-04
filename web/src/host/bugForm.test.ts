import { describe, expect, it } from 'vitest';
import { bugFormUrl } from './bugForm';

describe('bugFormUrl', () => {
  it('fills in the bug form by its field ids, leaving out what is empty', () => {
    const url = new URL(bugFormUrl({ repo: 'o/r', title: ' Board freezes ', whatHappened: 'It stops.\n& then', expected: '', steps: '1. Drag', version: '0.1.42' }));
    expect(url.origin + url.pathname).toBe('https://github.com/o/r/issues/new');
    expect(Object.fromEntries(url.searchParams)).toEqual({
      template: 'bug.yml', title: 'Board freezes', 'what-happened': 'It stops.\n& then', steps: '1. Drag', version: '0.1.42',
    });
  });

  it('adds the OS when it is shared and keeps long text short enough for GitHub', () => {
    const url = new URL(bugFormUrl({ repo: 'o/r', title: 'x', whatHappened: 'a'.repeat(5000), expected: '', steps: '', version: '1', os: 'macos (aarch64)' }));
    expect(url.searchParams.get('os')).toBe('macos (aarch64)');
    const what = url.searchParams.get('what-happened')!;
    expect(what).toHaveLength(1500);
    expect(what.endsWith('…')).toBe(true);
  });
});
