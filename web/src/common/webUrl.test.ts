import { describe, expect, it } from 'vitest';
import { isWebUrl } from './webUrl';

describe('isWebUrl', () => {
  it('allows http and https addresses', () => {
    expect(isWebUrl('https://example.com/a')).toBe(true);
    expect(isWebUrl('HTTP://example.com')).toBe(true);
  });

  it('refuses other schemes and things that are not addresses', () => {
    for (const u of ['javascript:alert(1)', 'data:text/html,x', 'file:///etc/passwd', '//evil.example', 'evil.example', ''])
      expect(isWebUrl(u)).toBe(false);
  });
});
