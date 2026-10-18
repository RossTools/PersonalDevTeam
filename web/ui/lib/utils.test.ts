import { describe, expect, it } from 'vitest';
import { cn } from './utils';

describe('cn', () => {
  it('treats our type sizes as font sizes, not colours', () => {
    expect(cn('text-meta', 'text-muted-foreground')).toBe('text-meta text-muted-foreground');
    expect(cn('text-sm', 'text-2xs')).toBe('text-2xs');
  });
  it('treats control heights as spacing', () => {
    expect(cn('h-8', 'h-control-sm')).toBe('h-control-sm');
  });
  it('treats our shadows as shadows', () => {
    expect(cn('shadow-md', 'shadow-popover')).toBe('shadow-popover');
  });
});
