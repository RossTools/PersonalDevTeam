import { describe, expect, it } from 'vitest';
import { NEWLINE, isShiftEnter, promptSequence, shouldOpenLink } from './input';

const key = (init: Partial<KeyboardEvent>) => ({ key: 'Enter', shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, ...init }) as KeyboardEvent;
const click = (init: Partial<MouseEvent>) => ({ metaKey: false, ctrlKey: false, ...init }) as MouseEvent;

describe('isShiftEnter', () => {
  it('matches Shift+Enter only', () => {
    expect(isShiftEnter(key({ shiftKey: true }))).toBe(true);
    expect(isShiftEnter(key({}))).toBe(false);
    expect(isShiftEnter(key({ shiftKey: true, ctrlKey: true }))).toBe(false);
    expect(isShiftEnter(key({ shiftKey: true, metaKey: true }))).toBe(false);
    expect(isShiftEnter(key({ shiftKey: true, key: 'a' }))).toBe(false);
  });

  it('inserts a newline as Meta+Enter, which agents read as a line break', () => {
    expect(NEWLINE).toBe('\x1b\r');
  });
});

describe('promptSequence', () => {
  it('pastes the text with line breaks as Enter, bracketed when the program asked for it, then submits', () => {
    expect(promptSequence('one\ntwo\r\nthree', true)).toBe('\x1b[200~one\rtwo\rthree\x1b[201~\r');
    expect(promptSequence('one\ntwo', false)).toBe('one\rtwo\r');
  });

  it('strips escape characters so the text cannot end the paste early', () => {
    expect(promptSequence('a\x1b[201~b', true)).toBe('\x1b[200~a[201~b\x1b[201~\r');
  });
});

describe('shouldOpenLink', () => {
  it('opens web links on Cmd/Ctrl+click, or any tap on a touch screen', () => {
    expect(shouldOpenLink(click({ metaKey: true }), 'https://example.com', false)).toBe(true);
    expect(shouldOpenLink(click({ ctrlKey: true }), 'http://example.com/x', false)).toBe(true);
    expect(shouldOpenLink(click({}), 'https://example.com', false)).toBe(false);
    expect(shouldOpenLink(click({}), 'https://example.com', true)).toBe(true);
  });

  it('never opens other schemes', () => {
    expect(shouldOpenLink(click({ metaKey: true }), 'file:///etc/passwd', false)).toBe(false);
    expect(shouldOpenLink(click({ metaKey: true }), 'javascript:alert(1)', false)).toBe(false);
    expect(shouldOpenLink(click({ metaKey: true }), 'not a url', false)).toBe(false);
  });
});
