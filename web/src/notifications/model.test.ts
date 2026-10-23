import { describe, expect, it } from 'vitest';
import { notification } from './fixtures';
import { presentation, reaches, STALE_MS, type Present } from './model';

const now = 1_000_000;
const here: Present = { visible: true, desktopAllowed: true, leader: true, onScreenSession: null, onScreenInbox: null, now };
const n = (over: Parameters<typeof notification>[1]) => notification('n', { createdAt: now, ...over });

describe('what a row does on this page', () => {
  it('toasts while visible and goes to the desktop while hidden, by its Show', () => {
    expect(presentation(n({ show: 'toast' }), here)).toBe('toast');
    expect(presentation(n({ show: 'desktop' }), here)).toBe('toast');
    expect(presentation(n({ show: 'inbox' }), here)).toBeNull();
    const hidden = { ...here, visible: false };
    expect(presentation(n({ show: 'desktop' }), hidden)).toBe('desktop');
    expect(presentation(n({ show: 'toast' }), hidden)).toBeNull();
  });

  it('needs the device to allow it and this tab to hold the lock', () => {
    const hidden = { ...here, visible: false };
    expect(presentation(n({ show: 'desktop' }), { ...hidden, desktopAllowed: false })).toBeNull();
    expect(presentation(n({ show: 'desktop' }), { ...hidden, leader: false })).toBeNull();
  });

  it('skips what is on screen, late rows and cleared ones', () => {
    expect(presentation(n({ show: 'toast', sessionId: 's' }), { ...here, onScreenSession: 's' })).toBeNull();
    expect(presentation(n({ show: 'toast', inboxItemId: 'i' }), { ...here, onScreenInbox: 'i' })).toBeNull();
    expect(presentation(n({ show: 'toast', sessionId: 's' }), { ...here, onScreenSession: 'other' })).toBe('toast');
    expect(presentation(n({ show: 'desktop', createdAt: now - STALE_MS - 1 }), { ...here, visible: false })).toBeNull();
    expect(presentation(n({ show: 'desktop', clearedAt: now }), { ...here, visible: false })).toBeNull();
  });

  it('orders the levels', () => {
    expect(reaches('desktop', 'toast')).toBe(true);
    expect(reaches('dot', 'inbox')).toBe(false);
  });
});
