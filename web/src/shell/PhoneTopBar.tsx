// The phone's top bar (PDT-112): ☰ or ←, the screen's title, 🔔 Needs you, ⌕ Search, ⋯ the screen's actions.
import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import { ArrowLeft, Bell, Menu, Search } from 'lucide-react';
import { IconButton, MenuButton, type MenuEntry } from '@ui';
import { InstanceBadge } from './InstanceBadge';

/** A screen's ⋯ entries, built when the menu opens. */
export type TopBarActions = () => MenuEntry[];

/** Sets the screen's own ⋯ entries (null: none). AppShell provides it on phones. */
export const TopBarActionsContext = createContext<(actions: { entries: TopBarActions } | null) => void>(() => {});

/**
 * Puts a screen's actions under the top bar's ⋯ while it is shown (phones). `entries` is read when the menu opens, so
 * it may change on every render.
 */
export function useTopBarActions(entries: TopBarActions | null) {
  const set = useContext(TopBarActionsContext);
  const ref = useRef(entries);
  ref.current = entries;
  const has = entries !== null;
  useEffect(() => {
    if (!has) return;
    set({ entries: () => ref.current?.() ?? [] });
    return () => set(null);
  }, [has, set]);
}

export function PhoneTopBar({ detail, onMenu, menuOpen, onBack, title, needsYou, onInbox, onSearch, actions }: {
  /** A detail screen: ← (Back) instead of ☰. */
  detail: boolean;
  onMenu: () => void;
  menuOpen: boolean;
  onBack: () => void;
  /** Text, or a button (a project picker) for screens that have one. */
  title: ReactNode;
  needsYou: number;
  onInbox: () => void;
  onSearch: () => void;
  /** ⋯ entries: the shell's for this screen, then the screen's own. */
  actions: TopBarActions | null;
}) {
  const shown = needsYou > 99 ? '99+' : needsYou;
  return (
    <header aria-label="Top bar" className="flex min-w-0 items-center gap-1 border-b border-border bg-card px-1.5 py-1">
      {detail ? (
        <IconButton label="Back" noTooltip onClick={onBack} className="size-11"><ArrowLeft aria-hidden /></IconButton>
      ) : (
        <IconButton label="Menu" noTooltip aria-haspopup="dialog" aria-expanded={menuOpen} onClick={onMenu} className="size-11"><Menu aria-hidden /></IconButton>
      )}
      <h1 className="flex min-w-0 flex-1 items-center text-sm font-medium">{typeof title === 'string' ? <span className="truncate px-1">{title}</span> : title}</h1>
      <InstanceBadge compact />
      <IconButton label={needsYou > 0 ? `Needs you (${needsYou})` : 'Needs you'} noTooltip onClick={onInbox} className="relative size-11 text-muted-foreground">
        <Bell aria-hidden />
        {needsYou > 0 && (
          <span aria-hidden className="absolute top-1 right-0.5 min-w-4 rounded-full bg-primary px-1 text-2xs leading-4 font-medium text-primary-foreground tabular-nums">
            {shown}
          </span>
        )}
      </IconButton>
      <IconButton label="Search" noTooltip onClick={onSearch} className="size-11 text-muted-foreground"><Search aria-hidden /></IconButton>
      {actions && <MenuButton label="Screen actions" entries={actions} className="size-11" />}
    </header>
  );
}
