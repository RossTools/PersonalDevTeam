import { describe, expect, it } from 'vitest';
import { isTicketKey, looksLikeKey } from './model';

describe('ticket keys', () => {
  it('isTicketKey takes a key in any case, never an id or a number', () => {
    for (const k of ['PDT-12', 'pdt-12', 'A1-3']) expect(isTicketKey(k)).toBe(true);
    for (const k of ['01a1a476-7269', '12', '#12', 'PDT-', '-12', 'PDT-12 ', '1PDT-2']) expect(isTicketKey(k)).toBe(false);
  });

  it('looksLikeKey also takes a bare number for Go to', () => {
    for (const q of ['PDT-12', ' pdt-12 ', '#12', '12']) expect(looksLikeKey(q)).toBe(true);
    for (const q of ['PDT', 'twelve', '#']) expect(looksLikeKey(q)).toBe(false);
  });
});
