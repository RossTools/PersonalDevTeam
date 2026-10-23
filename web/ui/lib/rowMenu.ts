// Row menus for List and DataList: right-click, long press (touch) and Shift+F10 / the Menu key all call a row's
// `onMenu`. Internal to web/ui (not exported from index.ts).
import { createContext, useContext, useEffect, useRef, type KeyboardEvent, type MouseEvent, type PointerEvent } from 'react';

/** Where to open a row's menu (viewport coordinates). */
export interface MenuPoint {
  x: number;
  y: number;
}

/** Shift+F10 or the Menu key. */
export const isKeyMenu = (e: KeyboardEvent) => e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey);

/** Opens the menu of the row holding focus (from a keydown inside the list); true when it did. */
export function openFocusedRowMenu(e: KeyboardEvent, menus: Map<string, (p: MenuPoint) => void>): boolean {
  if (!isKeyMenu(e)) return false;
  const row = (e.target as HTMLElement).closest<HTMLElement>('[data-key]');
  const open = row && menus.get(row.dataset.key ?? '');
  if (!row || !open) return false;
  e.preventDefault();
  const r = row.getBoundingClientRect();
  open({ x: r.left + 8, y: r.bottom });
  return true;
}

const LONG_PRESS_MS = 500;
/** The open-menu callbacks of a list's rows, by row id (the list provides it; Shift+F10 looks rows up in it). */
export const RowMenus = createContext<Map<string, (p: MenuPoint) => void> | null>(null);

/** Told when a touch long press opened a row's menu (React Aria also selects the row on a long press; the list undoes that). */
export const RowLongPress = createContext<((id: string) => void) | null>(null);

/** Right-click and long-press (touch) handlers for a row with a menu; registers it for Shift+F10. */
export function useRowMenu(id: string, onMenu: ((p: MenuPoint) => void) | undefined) {
  const menus = useContext(RowMenus);
  const longPressed = useContext(RowLongPress);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fired = useRef(false);
  const start = useRef<{ x: number; y: number } | null>(null);
  useEffect(() => {
    if (!menus || !onMenu) return;
    menus.set(id, onMenu);
    return () => { if (menus.get(id) === onMenu) menus.delete(id); };
  }, [menus, id, onMenu]);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  if (!onMenu) return {};
  const cancel = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  return {
    onContextMenu: (e: MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      // Android Chrome also fires contextmenu on a long press; the timer already opened the menu.
      if (fired.current) { fired.current = false; return; }
      onMenu({ x: e.clientX, y: e.clientY });
    },
    onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
      fired.current = false;
      if (e.pointerType !== 'touch') return;
      const at = { x: e.clientX, y: e.clientY };
      const { currentTarget: row, pointerId } = e;
      start.current = at;
      cancel();
      timer.current = setTimeout(() => {
        timer.current = null;
        fired.current = true;
        // End React Aria's press on the row: iOS sends no click after a long press, and without one React Aria
        // fires the row's action itself when the finger lifts, over the menu just opened.
        row.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true, pointerId, pointerType: 'touch' }));
        longPressed?.(id);
        onMenu(at);
      }, LONG_PRESS_MS);
    },
    onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
      const s = start.current;
      if (timer.current && s && Math.hypot(e.clientX - s.x, e.clientY - s.y) > 10) cancel();
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
  };
}
