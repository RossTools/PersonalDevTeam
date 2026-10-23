import { describe, expect, it } from 'vitest';
import { applyPending, clampFontSize, pressBarKey, tapCtrl, tapPrefix, withCtrl, wordAt } from './keyBar';

describe('withCtrl', () => {
  it('turns letters of either case into control bytes', () => {
    expect(withCtrl('c')).toBe('\x03');
    expect(withCtrl('C')).toBe('\x03');
    expect(withCtrl('r')).toBe('\x12');
    expect(withCtrl('a')).toBe('\x01');
    expect(withCtrl('z')).toBe('\x1a');
  });

  it('maps the punctuation a terminal maps', () => {
    expect(withCtrl('[')).toBe('\x1b');
    expect(withCtrl(']')).toBe('\x1d');
    expect(withCtrl('@')).toBe('\x00');
    expect(withCtrl(' ')).toBe('\x00');
    expect(withCtrl('?')).toBe('\x7f');
  });

  it('modifies arrows like xterm does', () => {
    expect(withCtrl('\x1b[A')).toBe('\x1b[1;5A');
    expect(withCtrl('\x1bOD')).toBe('\x1b[1;5D');
  });

  it('has no Ctrl form for digits, text or other sequences', () => {
    expect(withCtrl('1')).toBeNull();
    expect(withCtrl('ab')).toBeNull();
    expect(withCtrl('\x1b[<0;3;4M')).toBeNull();
  });
});

describe('sticky Ctrl', () => {
  it('applies to the next key only', () => {
    let pending = tapCtrl(null);
    expect(pending).toBe('ctrl');
    const first = applyPending(pending, 'r');
    expect(first).toEqual({ send: '\x12', pending: null });
    pending = first.pending;
    expect(applyPending(pending, 'r')).toEqual({ send: 'r', pending: null });
  });

  it('works with a key bar arrow', () => {
    expect(applyPending('ctrl', '\x1b[C')).toEqual({ send: '\x1b[1;5C', pending: null });
  });

  it('lets go when tapped again', () => {
    expect(tapCtrl('ctrl')).toBeNull();
    expect(tapCtrl('prefix')).toBe('ctrl');
  });

  it('stays armed through input it does not apply to', () => {
    expect(applyPending('ctrl', '\x1b[<0;3;4M')).toEqual({ send: '\x1b[<0;3;4M', pending: 'ctrl' });
  });

  it('passes input through when nothing is armed', () => {
    expect(applyPending(null, 'r')).toEqual({ send: 'r', pending: null });
  });
});

describe('key bar keys', () => {
  it('send their bytes, with Ctrl when it is armed, and let go of it', () => {
    expect(pressBarKey(null, '\x1b')).toEqual({ send: '\x1b', pending: null });
    expect(pressBarKey('ctrl', '\x1b[A')).toEqual({ send: '\x1b[1;5A', pending: null });
    expect(pressBarKey('ctrl', '\t')).toEqual({ send: '\t', pending: null });
  });

  it('cancel the prefix without sending', () => {
    expect(pressBarKey('prefix', '\x1b')).toEqual({ pending: null });
  });
});

describe('the ⌃] prefix', () => {
  it('runs the next typed key as a PDT shortcut instead of sending it', () => {
    const armed = tapPrefix(null);
    expect(armed).toEqual({ pending: 'prefix' });
    expect(applyPending(armed.pending, 'n')).toEqual({ prefixKey: 'n', pending: null });
  });

  it('sends Ctrl+] when tapped twice', () => {
    expect(tapPrefix('prefix')).toEqual({ send: '\x1d', pending: null });
  });

  it('lets escape sequences and pastes through and stays armed', () => {
    expect(applyPending('prefix', '\x1b[A')).toEqual({ send: '\x1b[A', pending: 'prefix' });
    expect(applyPending('prefix', 'hello')).toEqual({ send: 'hello', pending: 'prefix' });
  });
});

describe('clampFontSize', () => {
  it('keeps sizes in range and whole', () => {
    expect(clampFontSize(4)).toBe(9);
    expect(clampFontSize(40)).toBe(24);
    expect(clampFontSize(14.4)).toBe(14);
    expect(clampFontSize(NaN)).toBe(13);
  });
});

describe('wordAt', () => {
  it('finds a word or path around a column', () => {
    const line = 'see web/src/main.tsx now';
    expect(wordAt(line, 8)).toEqual([4, 20]);
    expect(wordAt(line, 0)).toEqual([0, 3]);
  });

  it('is one cell on a space or past the end', () => {
    expect(wordAt('a b', 1)).toEqual([1, 2]);
    expect(wordAt('ab', 5)).toEqual([5, 6]);
  });
});
