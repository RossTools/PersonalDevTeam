import { describe, expect, it } from 'vitest';
import { cn } from './utils';

describe('cn', () => {
  it('treats our type sizes as font sizes, not colours', () => {
    expect(cn('text-meta', 'text-muted-foreground')).toBe('text-meta text-muted-foreground');
    expect(cn('text-sm', 'text-2xs')).toBe('text-2xs');
    expect(cn('text-heading font-semibold', 'text-foreground')).toBe('text-heading font-semibold text-foreground');
    expect(cn('text-heading', 'text-muted-foreground')).toBe('text-heading text-muted-foreground');
  });
  it('treats the page gutter as spacing', () => {
    expect(cn('px-3', 'px-gutter')).toBe('px-gutter');
    expect(cn('p-4', 'p-gutter')).toBe('p-gutter');
  });
  it('treats control heights as spacing', () => {
    expect(cn('h-8', 'h-control-sm')).toBe('h-control-sm');
  });
  it('treats our shadows as shadows', () => {
    expect(cn('shadow-md', 'shadow-popover')).toBe('shadow-popover');
  });
});
