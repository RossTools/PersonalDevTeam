import type { ReactNode } from 'react';
import { cn } from '@ui/lib/utils';
import { IconButton } from '@ui';
import { SEARCH, VIEWS, type ViewId } from './views';

const item = 'size-9 pointer-coarse:size-11';

/**
 * The views: a column on the left on desktop, a bar along the bottom on phones. `top` sits above the views (the DEV
 * badge on desktop); `onSearch` adds the phone's Search item, which opens the command palette.
 */
export function IconRail({ active, onSelect, onSearch, top }: {
  active: ViewId;
  onSelect: (v: ViewId) => void;
  onSearch?: () => void;
  top?: ReactNode;
}) {
  const view = ({ id, label, icon: Icon }: (typeof VIEWS)[number]) => (
    <IconButton
      key={id}
      label={label}
      aria-current={active === id ? 'page' : undefined}
      onClick={() => onSelect(id)}
      className={cn(item, active === id ? 'bg-accent text-foreground' : 'text-muted-foreground')}
    >
      <Icon aria-hidden />
    </IconButton>
  );
  const settings = VIEWS.find((v) => v.id === 'settings')!;
  return (
    <nav aria-label="Views" className="flex items-center justify-around gap-1 border-t border-border bg-card p-1.5 md:flex-col md:justify-start md:border-t-0 md:border-r">
      {top && <div className="flex justify-center pb-1">{top}</div>}
      {VIEWS.filter((v) => v !== settings).map(view)}
      {onSearch && (
        <IconButton label={SEARCH.label} onClick={onSearch} className={cn(item, 'text-muted-foreground')}>
          <SEARCH.icon aria-hidden />
        </IconButton>
      )}
      {view(settings)}
    </nav>
  );
}
