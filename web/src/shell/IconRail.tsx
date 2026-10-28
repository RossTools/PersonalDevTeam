import { useState, type ReactNode } from 'react';
import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, IconButton, type ThemePref } from '@ui';
import { ThemeSwitch } from './ThemeSwitch';
import { VIEWS, type ViewId } from './views';

const item = 'size-9 pointer-coarse:size-11';
const wide = 'h-9 w-full justify-start gap-2 px-2.5 font-normal pointer-coarse:h-11';

/** Whether the desktop rail shows each view's name, in this browser (localStorage; any failure reads as collapsed). */
export const RAIL_EXPANDED_KEY = 'pdt.rail.expanded';

function readExpanded(): boolean {
  try {
    return localStorage.getItem(RAIL_EXPANDED_KEY) === 'true';
  } catch {
    return false;
  }
}

function writeExpanded(expanded: boolean) {
  try {
    localStorage.setItem(RAIL_EXPANDED_KEY, String(expanded));
  } catch {
    // Private window or blocked storage: the rail keeps its state for this visit only.
  }
}

/**
 * The views in a column on the left (desktop; phones have the drawer, PDT-112). `top` sits above the views (the DEV
 * badge). At the bottom, the theme switch (with `theme`), and a toggle that expands the column to show each view's name
 * beside its icon.
 */
export function IconRail({ active, onSelect, top, counts, dots, theme }: {
  active: ViewId;
  onSelect: (v: ViewId) => void;
  top?: ReactNode;
  /** A count badge on a view's item (the Inbox's open items). */
  counts?: Partial<Record<ViewId, number>>;
  /** A dot on a view's item with no count to show (the Inbox's unread Updates, the PM's unread reply). */
  dots?: Partial<Record<ViewId, boolean>>;
  /** The theme choice and its setter (AppShell's `useThemePref`, shared with the command palette). */
  theme?: { value: ThemePref; onChange: (pref: ThemePref) => void };
}) {
  const [expandedPref, setExpandedPref] = useState(readExpanded);
  const expanded = expandedPref;
  const toggle = () => {
    setExpandedPref(!expandedPref);
    writeExpanded(!expandedPref);
  };
  // Beside the column, so it never covers the next button.
  const tooltipPlacement = 'right';
  const view = ({ id, label, icon: Icon, dot: dotName }: (typeof VIEWS)[number]) => {
    const n = counts?.[id] ?? 0;
    const dot = n === 0 && !!dots?.[id];
    const name = n > 0 ? `${label} (${n})` : dot ? `${label} (${dotName ?? 'new'})` : label;
    const shown = n > 99 ? '99+' : n;
    const current = active === id ? 'page' : undefined;
    const tone = active === id ? 'bg-accent text-foreground' : 'text-muted-foreground';
    if (expanded) {
      return (
        <Button key={id} variant="ghost" aria-label={name} aria-current={current} onClick={() => onSelect(id)} className={cn(wide, tone)}>
          <Icon aria-hidden />
          <span className="min-w-0 flex-1 truncate text-left">{label}</span>
          {n > 0 && (
            <span aria-hidden className="min-w-4 rounded-full bg-primary px-1 text-center text-2xs leading-4 font-medium text-primary-foreground tabular-nums">
              {shown}
            </span>
          )}
          {dot && <span aria-hidden className="size-2 rounded-full bg-primary" />}
        </Button>
      );
    }
    return (
      <IconButton
        key={id}
        label={name}
        tooltipPlacement={tooltipPlacement}
        aria-current={current}
        onClick={() => onSelect(id)}
        className={cn('relative', item, tone)}
      >
        <Icon aria-hidden />
        {n > 0 && (
          <span aria-hidden className="absolute -top-0.5 -right-0.5 min-w-4 rounded-full bg-primary px-1 text-2xs leading-4 font-medium text-primary-foreground tabular-nums">
            {shown}
          </span>
        )}
        {dot && <span aria-hidden className="absolute top-1 right-1 size-2 rounded-full bg-primary" />}
      </IconButton>
    );
  };
  const settings = VIEWS.find((v) => v.id === 'settings')!;
  return (
    <nav
      aria-label="Views"
      className={cn(
        'flex flex-col items-center gap-1 border-r border-border bg-card p-1.5',
        expanded && 'w-44 shrink-0 items-stretch',
      )}
    >
      {top && <div className="flex justify-center pb-1">{top}</div>}
      {VIEWS.filter((v) => v !== settings).map(view)}
      {view(settings)}
      <div className="mt-auto flex flex-col items-stretch gap-1 pt-1">
        {theme && <ThemeSwitch {...theme} layout={expanded ? 'icons' : 'column'} />}
        {expanded ? (
          <Button variant="ghost" aria-label="Collapse side rail" aria-expanded onClick={toggle} className={cn(wide, 'text-muted-foreground')}>
            <PanelLeftClose aria-hidden />
            <span className="min-w-0 flex-1 truncate text-left">Collapse</span>
          </Button>
        ) : (
          <IconButton label="Expand side rail" aria-expanded={false} tooltipPlacement={tooltipPlacement} onClick={toggle} className={cn(item, 'text-muted-foreground')}>
            <PanelLeftOpen aria-hidden />
          </IconButton>
        )}
      </div>
    </nav>
  );
}
