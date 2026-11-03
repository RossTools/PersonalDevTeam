import { createCn } from 'cn/config';

/**
 * clsx + tailwind-merge that knows PDT's theme names (src/index.css), so they win or lose against the classes they
 * replace: `text-meta`/`text-2xs`/`text-heading`/`text-title` are font sizes (not text colours), `h-control-sm` etc. and `px-gutter` are spacing, `shadow-popover`,
 * `shadow-dialog` and `shadow-card` are shadows. Without this, `cn('text-meta', 'text-muted-foreground')` drops the size.
 * Everything in web/ui and web/src imports cn from here, never from the bare cn package (src/lint.test.ts).
 */
export const cn = createCn({
  extend: {
    theme: {
      text: ['2xs', 'meta', 'heading', 'title'],
      spacing: ['control-sm', 'control-md', 'control-touch', 'control-lg', 'gutter'],
      shadow: ['popover', 'dialog', 'card'],
    },
  },
});
