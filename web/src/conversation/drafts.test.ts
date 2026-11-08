// The drafts keep module state (the page's memory), so each test uses its own session ids.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { loadDraft, loadImageDrafts, saveDraft, saveImageDrafts, type ImageDraft } from './drafts';

afterEach(() => {
  vi.restoreAllMocks();
  sessionStorage.clear();
});

describe('message drafts', () => {
  it('keeps unsent text per session in this tab’s storage, and forgets it once empty', () => {
    expect(loadDraft('s1')).toBe('');
    saveDraft('s1', 'Hello');
    saveDraft('s2', 'Other');
    expect(loadDraft('s1')).toBe('Hello');
    expect(sessionStorage.getItem('pdt.conversation.draft.s1')).toBe('Hello');
    saveDraft('s1', '');
    expect(loadDraft('s1')).toBe('');
    expect(sessionStorage.getItem('pdt.conversation.draft.s1')).toBeNull();
    expect(loadDraft('s2')).toBe('Other');
  });

  it('reads what another page of this tab stored', () => {
    sessionStorage.setItem('pdt.conversation.draft.s3', 'From before a reload');
    expect(loadDraft('s3')).toBe('From before a reload');
  });

  it('keeps the text in the page’s memory when storage is blocked', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => { throw new Error('blocked'); });
    expect(() => saveDraft('s4', 'Typed')).not.toThrow();
    expect(loadDraft('s4')).toBe('Typed');
    saveDraft('s4', '');
    expect(loadDraft('s4')).toBe('');
  });
});

describe('image drafts', () => {
  const image = (key: string): ImageDraft => ({ key, name: `${key}.png`, url: `blob:${key}`, id: `up-${key}` });

  it('keeps uploaded images per session until the list is emptied', () => {
    expect(loadImageDrafts('i1')).toEqual([]);
    saveImageDrafts('i1', [image('a'), image('b')]);
    saveImageDrafts('i2', [image('c')]);
    expect(loadImageDrafts('i1').map((i) => i.key)).toEqual(['a', 'b']);
    saveImageDrafts('i1', []);
    expect(loadImageDrafts('i1')).toEqual([]);
    expect(loadImageDrafts('i2').map((i) => i.key)).toEqual(['c']);
  });
});
