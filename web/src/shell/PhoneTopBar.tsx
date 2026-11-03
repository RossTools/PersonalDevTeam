// The phone's top bar (PDT-112): ☰ or ←, the screen's title, 🔔 Needs you, ⌕ Search, ☑ Todos, ⋯ the screen's actions.
import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import { ArrowLeft, Bell, ListChecks, Menu, Search } from 'lucide-react';
import { Badge, Button, IconButton, MenuButton, type MenuEntry } from '@ui';
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

/** An edit in progress on the screen (a ticket's title or description): the top bar shows Cancel and Save for it. */
export interface TopBarEdit {
  /** Save's state: the draft can be saved, and a save is under way. */
  canSave: boolean;
  saving: boolean;
  onSave: () => void;
  onCancel: () => void;
}

/** Sets the screen's edit (null: none). AppShell provides it on phones. */
export const TopBarEditContext = createContext<(edit: TopBarEdit | null) => void>(() => {});

/**
 * Puts Cancel and Save in the top bar while `edit` is set (phones). The handlers are read when tapped, so they may
 * change on every render; the bar re-renders only when Save's state does.
 */
export function useTopBarEdit(edit: TopBarEdit | null) {
  const set = useContext(TopBarEditContext);
  const ref = useRef(edit);
  ref.current = edit;
  const on = edit !== null;
  const canSave = edit?.canSave ?? false;
  const saving = edit?.saving ?? false;
  useEffect(() => {
    if (!on) return;
    set({ canSave, saving, onSave: () => ref.current?.onSave(), onCancel: () => ref.current?.onCancel() });
  }, [on, canSave, saving, set]);
  useEffect(() => (on ? () => set(null) : undefined), [on, set]);
}

export function PhoneTopBar({ detail, onMenu, menuOpen, onBack, title, needsYou, onInbox, onSearch, onTodos, actions, edit }: {
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
  /** Opens the todo list (the desktop's right panel) in a sheet. */
  onTodos?: () => void;
  /** ⋯ entries: the shell's for this screen, then the screen's own. */
  actions: TopBarActions | null;
  /** An edit on the screen: Cancel and Save take the bar's ends. */
  edit?: TopBarEdit | null;
}) {
  if (edit) {
    return (
      <header aria-label="Top bar" className="flex min-w-0 items-center gap-1 border-b border-border bg-card px-1.5 py-1">
        <Button variant="ghost" className="h-control-touch pointer-coarse:h-control-touch" disabled={edit.saving} onClick={edit.onCancel}>Cancel</Button>
        <h1 className="flex min-w-0 flex-1 items-center text-sm font-medium">{typeof title === 'string' ? <span className="truncate px-1">{title}</span> : title}</h1>
        <Button variant="primary" className="h-control-touch pointer-coarse:h-control-touch" pending={edit.saving} disabled={!edit.canSave} onClick={edit.onSave}>Save</Button>
      </header>
    );
  }
  return (
    <header aria-label="Top bar" className="flex min-w-0 items-center gap-1 border-b border-border bg-card px-1.5 py-1">
      {detail ? (
        <IconButton label="Back" noTooltip onClick={onBack} className="size-control-touch"><ArrowLeft aria-hidden /></IconButton>
      ) : (
        <IconButton label="Menu" noTooltip aria-haspopup="dialog" aria-expanded={menuOpen} onClick={onMenu} className="size-control-touch"><Menu aria-hidden /></IconButton>
      )}
      <h1 className="flex min-w-0 flex-1 items-center text-sm font-medium">{typeof title === 'string' ? <span className="truncate px-1">{title}</span> : title}</h1>
      <InstanceBadge compact />
      <IconButton label={needsYou > 0 ? `Needs you (${needsYou})` : 'Needs you'} noTooltip onClick={onInbox} className="relative size-control-touch text-muted-foreground">
        <Bell aria-hidden />
        {needsYou > 0 && <Badge aria-hidden tone="solid" size="sm" count={needsYou} className="absolute top-1 right-0.5" />}
      </IconButton>
      <IconButton label="Search" noTooltip onClick={onSearch} className="size-control-touch text-muted-foreground"><Search aria-hidden /></IconButton>
      {onTodos && <IconButton label="Todos" noTooltip aria-haspopup="dialog" onClick={onTodos} className="size-control-touch text-muted-foreground"><ListChecks aria-hidden /></IconButton>}
      {actions && <MenuButton label="Screen actions" entries={actions} className="size-control-touch" />}
    </header>
  );
}
