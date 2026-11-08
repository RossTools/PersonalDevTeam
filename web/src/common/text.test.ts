import { describe, expect, it } from 'vitest';
import { capitalize, formatBytes, listText, plural } from './text';

const GB = 1024 ** 3;

describe('plural', () => {
  it('adds an s for anything but one, or takes the plural given', () => {
    expect(plural(1, 'file')).toBe('1 file');
    expect(plural(0, 'file')).toBe('0 files');
    expect(plural(3, 'review comment')).toBe('3 review comments');
    expect(plural(2, 'entry', 'entries')).toBe('2 entries');
  });
});

describe('listText', () => {
  it('joins with commas and a last "and" (or the word given)', () => {
    expect(listText([])).toBe('');
    expect(listText(['a'])).toBe('a');
    expect(listText(['a', 'b'])).toBe('a and b');
    expect(listText(['a', 'b', 'c'], 'or')).toBe('a, b or c');
  });
});

describe('capitalize', () => {
  it('upper-cases the first letter only', () => {
    expect(capitalize('merge conflict')).toBe('Merge conflict');
    expect(capitalize('')).toBe('');
  });
});

describe('formatBytes', () => {
  it('says sizes for people', () => {
    expect(formatBytes(512)).toBe('512 bytes');
    expect(formatBytes(1.5 * GB)).toBe('1.5 GB');
    expect(formatBytes(23 * GB)).toBe('23 GB');
  });
});
