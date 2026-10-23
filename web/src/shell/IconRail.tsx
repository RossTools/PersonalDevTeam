import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { IconButton } from '@ui';
import { SEARCH, VIEWS, type ViewId } from './views';

const item = 'size-9 pointer-coarse:size-11';

/**
 * The views: a column on the left on desktop, a bar along the bottom on phones. `top` sits above the views (the DEV
 * badge on desktop); `onSearch` makes it the phone rail: the Search item (opens the command palette) and only the views
 * meant for phones.
 */
export function IconRail({ active, onSelect, onSearch, top, counts, dots }: {
  active: ViewId;
  onSelect: (v: ViewId) => void;
  onSearch?: () => void;
  top?: ReactNode;
  /** A count badge on a view's item (the Inbox's open items). */
  counts?: Partial<Record<ViewId, number>>;
  /** A dot on a view's item with no count to show (the Inbox's unread Updates). */
  dots?: Partial<Record<ViewId, boolean>>;
}) {
  // Beside the column on desktop, so it never covers the next button; above the bar on phones.
  const tooltipPlacement = onSearch ? 'top' : 'right';
  const view = ({ id, label, icon: Icon }: (typeof VIEWS)[number]) => {
    const n = counts?.[id] ?? 0;
    const dot = n === 0 && !!dots?.[id];
    return (
      <IconButton
        key={id}
        label={n > 0 ? `${label} (${n})` : dot ? `${label} (new updates)` : label}
        tooltipPlacement={tooltipPlacement}
        aria-current={active === id ? 'page' : undefined}
        onClick={() => onSelect(id)}
        className={cn('relative', item, active === id ? 'bg-accent text-foreground' : 'text-muted-foreground')}
      >
        <Icon aria-hidden />
        {n > 0 && (
          <span aria-hidden className="absolute -top-0.5 -right-0.5 min-w-4 rounded-full bg-primary px-1 text-2xs leading-4 font-medium text-primary-foreground tabular-nums">
            {n > 99 ? '99+' : n}
          </span>
        )}
        {dot && <span aria-hidden className="absolute top-1 right-1 size-2 rounded-full bg-primary" />}
      </IconButton>
    );
  };
  const settings = VIEWS.find((v) => v.id === 'settings')!;
  return (
    <nav aria-label="Views" className="flex items-center justify-around gap-1 border-t border-border bg-card p-1.5 md:flex-col md:justify-start md:border-t-0 md:border-r">
      {top && <div className="flex justify-center pb-1">{top}</div>}
      {VIEWS.filter((v) => v !== settings && (!onSearch || v.phone !== false)).map(view)}
      {onSearch && (
        <IconButton label={SEARCH.label} tooltipPlacement={tooltipPlacement} onClick={onSearch} className={cn(item, 'text-muted-foreground')}>
          <SEARCH.icon aria-hidden />
        </IconButton>
      )}
      {view(settings)}
    </nav>
  );
}
