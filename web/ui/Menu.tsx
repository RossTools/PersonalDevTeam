import { isValidElement, useCallback, useContext, useState, type FormEvent, type ReactElement, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { cn } from 'cn';
import {
  Header,
  Menu as AriaMenu,
  MenuContext,
  MenuSection,
  MenuTrigger,
  Pressable,
  RootMenuTriggerStateContext,
  SubmenuTrigger,
  type MenuProps,
} from 'react-aria-components';
import { EllipsisIcon } from 'lucide-react';
import {
  DropdownMenuItem,
  DropdownMenuPopover,
  DropdownMenuSeparator,
  DropdownMenuSubTrigger,
} from './primitives/dropdown-menu';
import { IconButton } from './IconButton';
import { Kbd } from './Kbd';
import { pressableChild } from './lib/pressableChild';
import type { MenuPoint } from './List';

/** One entry of a menu. Every menu in PDT (right-click, long press, "⋯") is a list of these. */
export type MenuEntry =
  | { label: string; onSelect: () => void; disabled?: boolean; danger?: boolean; shortcut?: string; icon?: ReactNode }
  | { heading: string }
  | { separator: true }
  | { submenu: string; entries: MenuEntry[]; disabled?: boolean; icon?: ReactNode }
  /** A one-line text field ("New workspace…"): shown as an item; choosing it turns the menu into the field. */
  | { input: string; onSubmit: (value: string) => void };

type InputEntry = Extract<MenuEntry, { input: string }>;

const LIST = 'max-h-[inherit] min-w-40 max-w-72 overflow-x-hidden overflow-y-auto outline-hidden';
const ITEM = 'pointer-coarse:min-h-control-touch';

function ItemBody({ icon, label, shortcut }: { icon?: ReactNode; label: string; shortcut?: string }) {
  return (
    <>
      {icon}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {shortcut && <Kbd className="ml-auto">{shortcut}</Kbd>}
    </>
  );
}

/** Headings start a section (React Aria keeps headers inside sections); a separator ends one. */
function renderEntries(entries: MenuEntry[], onInput: (e: InputEntry) => void): ReactNode[] {
  const out: ReactNode[] = [];
  let section: { heading: string; key: string; items: ReactNode[] } | null = null;
  const flush = () => {
    if (!section) return;
    const s: { heading: string; key: string; items: ReactNode[] } = section;
    out.push(
      <MenuSection key={s.key} id={s.key}>
        <Header className="px-1.5 pt-1.5 pb-0.5 text-2xs font-medium tracking-wider text-muted-foreground uppercase">
          {s.heading}
        </Header>
        {s.items}
      </MenuSection>,
    );
    section = null;
  };
  const push = (n: ReactNode) => (section ? section.items.push(n) : out.push(n));
  entries.forEach((e, i) => {
    const key = `e${i}`;
    if ('heading' in e) {
      flush();
      section = { heading: e.heading, key, items: [] };
    } else if ('separator' in e) {
      flush();
      out.push(<DropdownMenuSeparator key={key} id={key} />);
    } else if ('submenu' in e) {
      push(
        <SubmenuTrigger key={key}>
          <DropdownMenuSubTrigger id={key} textValue={e.submenu} isDisabled={e.disabled} className={ITEM}>
            {e.icon}
            <span className="min-w-0 flex-1 truncate">{e.submenu}</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuPopover placement="end top" offset={0} crossOffset={-5}>
            <MenuBody label={e.submenu} entries={e.entries} />
          </DropdownMenuPopover>
        </SubmenuTrigger>,
      );
    } else if ('input' in e) {
      push(
        <DropdownMenuItem key={key} id={key} textValue={e.input} shouldCloseOnSelect={false} onAction={() => onInput(e)} className={ITEM}>
          <span className="min-w-0 flex-1 truncate text-muted-foreground">{e.input}</span>
        </DropdownMenuItem>,
      );
    } else {
      push(
        <DropdownMenuItem
          key={key}
          id={key}
          textValue={e.label}
          isDisabled={e.disabled}
          variant={e.danger ? 'destructive' : 'default'}
          onAction={e.onSelect}
          className={cn(ITEM, e.danger && 'text-destructive')}
        >
          <ItemBody icon={e.icon} label={e.label} shortcut={e.shortcut} />
        </DropdownMenuItem>,
      );
    }
  });
  flush();
  return out;
}

function InputForm({ entry, onDone }: { entry: InputEntry; onDone: () => void }) {
  const submit = (ev: FormEvent<HTMLFormElement>) => {
    ev.preventDefault();
    const v = new FormData(ev.currentTarget).get('v')?.toString().trim();
    if (!v) return;
    onDone();
    entry.onSubmit(v);
  };
  return (
    <form onSubmit={submit} className="p-0.5">
      <input
        name="v"
        aria-label={entry.input}
        placeholder={entry.input}
        autoFocus
        autoComplete="off"
        className="h-control-md w-56 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:h-control-touch"
      />
    </form>
  );
}

/**
 * The content of a menu popover: the entries (computed once per opening), or the text field of an input entry once
 * it's chosen. The menu is named by `label`, not by its trigger.
 */
function MenuBody({ label, entries }: { label: string; entries: MenuEntry[] | (() => MenuEntry[]) }) {
  const [list] = useState(entries);
  const [input, setInput] = useState<InputEntry | null>(null);
  const root = useContext(RootMenuTriggerStateContext);
  const ctx = useContext(MenuContext) as MenuProps<object> | null;
  if (input) return <InputForm entry={input} onDone={() => root?.close()} />;
  return (
    <MenuContext.Provider value={{ ...ctx, 'aria-labelledby': undefined }}>
      <AriaMenu aria-label={label} className={LIST}>
        {renderEntries(list, setInput)}
      </AriaMenu>
    </MenuContext.Provider>
  );
}

export interface ContextMenuProps {
  /** Called each time the menu opens, so the entries reflect the state at that moment. */
  entries: () => MenuEntry[];
  /** The menu's accessible name ("Tab Claude A"). */
  label: string;
  /**
   * The element the menu belongs to: a focusable DOM element (`<button>`, an element with an interactive role) or a
   * React Aria based component (our Button). One child.
   */
  children: ReactElement;
  /** No menu: right-click and long press do nothing special. */
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * A menu on right-click (and Shift+F10 / the context-menu key), long press on touch screens, opened at the pointer.
 * Escape closes it and focus returns to the element.
 */
export function ContextMenu({ entries, label, children, disabled, onOpenChange }: ContextMenuProps) {
  if (disabled) return children;
  return (
    <MenuTrigger trigger="contextMenu" onOpenChange={onOpenChange}>
      {pressableChild(children)}
      <DropdownMenuPopover placement="bottom start" offset={0}>
        <MenuBody label={label} entries={entries} />
      </DropdownMenuPopover>
    </MenuTrigger>
  );
}

export interface MenuButtonProps {
  entries: () => MenuEntry[];
  /** The menu's accessible name; also the default trigger's label ("More actions"). */
  label: string;
  /** A custom trigger (a Button, or a DOM button); defaults to a "⋯" icon button labelled `label`. */
  children?: ReactElement;
  /** Disables the default trigger (a custom trigger takes its own `disabled`). */
  disabled?: boolean;
  /** Default trigger size: md 28px (default) or sm 24px. */
  size?: 'sm' | 'md';
  placement?: 'bottom start' | 'bottom end' | 'top start' | 'top end';
  className?: string;
  onOpenChange?: (open: boolean) => void;
}

/** A button that opens a menu below it (the overflow "⋯"). Enter, Space, ArrowDown open it; Escape closes it. */
export function MenuButton({
  entries,
  label,
  children,
  disabled,
  size = 'md',
  placement = 'bottom end',
  className,
  onOpenChange,
}: MenuButtonProps) {
  const trigger = isValidElement(children) ? (
    pressableChild(children)
  ) : (
    <IconButton label={label} size={size} disabled={disabled} className={className}>
      <EllipsisIcon aria-hidden />
    </IconButton>
  );
  return (
    <MenuTrigger onOpenChange={onOpenChange}>
      {trigger}
      <DropdownMenuPopover placement={placement}>
        <MenuBody label={label} entries={entries} />
      </DropdownMenuPopover>
    </MenuTrigger>
  );
}

export interface MenuAtProps {
  open: boolean;
  /** Where the menu's top-left corner goes; it flips/shifts to stay on screen. */
  at: MenuPoint;
  /** An array, or a function called when the menu opens. */
  entries: MenuEntry[] | (() => MenuEntry[]);
  label: string;
  /** Escape, a click outside, or a chosen item. */
  onClose: () => void;
}

/**
 * A controlled menu opened at a point, for elements that can't be wrapped by `ContextMenu` (React Aria GridList rows
 * call `onMenu(at)` instead). Same entries, keys and look; focus returns to what was focused when it opened.
 */
export function MenuAt({ open, at, entries, label, onClose }: MenuAtProps) {
  // The menu anchors to an invisible, zero-size element at the point (React Aria positions popovers against a trigger).
  const anchor = (
    <Pressable>
      <span
        role="button"
        tabIndex={-1}
        aria-hidden
        style={{ position: 'fixed', left: at.x, top: at.y, width: 0, height: 0, overflow: 'hidden' }}
      />
    </Pressable>
  );
  return (
    <MenuTrigger isOpen={open} onOpenChange={(o) => !o && onClose()}>
      {createPortal(anchor, document.body)}
      <DropdownMenuPopover placement="bottom start" offset={0}>
        <MenuBody label={label} entries={entries} />
      </DropdownMenuPopover>
    </MenuTrigger>
  );
}

/**
 * One menu shared by many elements: `openMenu(at, label, entries)` from a handler (ListRow's `onMenu`), and render
 * `menu` once.
 */
export function useMenuAt(): {
  openMenu: (at: MenuPoint, label: string, entries: MenuEntry[] | (() => MenuEntry[])) => void;
  menu: ReactNode;
} {
  const [state, setState] = useState<{ at: MenuPoint; label: string; entries: MenuEntry[] | (() => MenuEntry[]); n: number; open: boolean }>({
    at: { x: 0, y: 0 },
    label: 'Menu',
    entries: [],
    n: 0,
    open: false,
  });
  const openMenu = useCallback(
    (at: MenuPoint, label: string, entries: MenuEntry[] | (() => MenuEntry[])) =>
      setState((s) => ({ at, label, entries, n: s.n + 1, open: true })),
    [],
  );
  const close = useCallback(() => setState((s) => ({ ...s, open: false })), []);
  const menu = <MenuAt key={state.n} open={state.open} at={state.at} label={state.label} entries={state.entries} onClose={close} />;
  return { openMenu, menu };
}
