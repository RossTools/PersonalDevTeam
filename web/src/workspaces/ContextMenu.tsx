import { useCallback, useLayoutEffect, useRef, useState, type MouseEvent, type TouchEvent } from 'react';
import { createPortal } from 'react-dom';

export type MenuEntry =
  | { label: string; onSelect: () => void; disabled?: boolean; danger?: boolean }
  | { heading: string }
  | { input: string; onSubmit: (value: string) => void };

interface OpenMenu {
  x: number;
  y: number;
  label: string;
  entries: MenuEntry[];
}

const LONG_PRESS_MS = 500;

function Menu({ menu, onClose }: { menu: OpenMenu; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: menu.x, top: menu.y });
  // Keep the whole menu on screen.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(menu.x, window.innerWidth - r.width - 4)),
      top: Math.max(4, Math.min(menu.y, window.innerHeight - r.height - 4)),
    });
    el.querySelector<HTMLElement>('[role=menuitem]:not([disabled]), input')?.focus();
  }, [menu]);
  const item = 'text-left px-2 py-1 rounded-pdt hover:bg-surface-2 focus:bg-surface-2 outline-none disabled:opacity-40';
  return createPortal(
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }} aria-hidden />
      <div
        ref={ref}
        role="menu"
        aria-label={menu.label}
        style={{ left: pos.left, top: pos.top }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') onClose();
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            const items = [...(ref.current?.querySelectorAll<HTMLElement>('[role=menuitem]:not([disabled])') ?? [])];
            const i = items.indexOf(document.activeElement as HTMLElement);
            items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
          }
        }}
        className="fixed z-50 min-w-48 max-w-72 rounded-pdt border border-border bg-surface shadow-lg p-1 flex flex-col gap-0.5 text-sm"
      >
        {menu.entries.map((e, i) =>
          'heading' in e ? (
            <p key={i} className="px-2 pt-1 text-2xs uppercase tracking-wider text-muted-foreground">{e.heading}</p>
          ) : 'input' in e ? (
            <form
              key={i}
              className="px-1"
              onSubmit={(ev) => {
                ev.preventDefault();
                const v = new FormData(ev.currentTarget).get('v')?.toString().trim();
                if (v) {
                  onClose();
                  e.onSubmit(v);
                }
              }}
            >
              <input name="v" aria-label={e.input} placeholder={e.input}
                className="w-full px-1 py-0.5 rounded-pdt text-xs bg-bg border border-border outline-none focus-visible:ring-2 focus-visible:ring-ring" />
            </form>
          ) : (
            <button
              key={i}
              type="button"
              role="menuitem"
              disabled={e.disabled}
              className={`${item} ${e.danger ? 'text-failed' : ''}`}
              onClick={() => { onClose(); e.onSelect(); }}
            >
              {e.label}
            </button>
          ),
        )}
      </div>
    </>,
    document.body,
  );
}

/**
 * A right-click menu at the pointer (a long press on touch screens). `bind(label, entries)` gives the handlers to
 * spread on the element; render `menu` once.
 */
export function useContextMenu() {
  const [open, setOpen] = useState<OpenMenu | null>(null);
  const press = useRef<{ timer: number; x: number; y: number } | null>(null);
  const cancel = () => {
    if (press.current) window.clearTimeout(press.current.timer);
    press.current = null;
  };
  const bind = useCallback((label: string, entries: () => MenuEntry[]) => ({
    onContextMenu: (e: MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setOpen({ x: e.clientX, y: e.clientY, label, entries: entries() });
    },
    onTouchStart: (e: TouchEvent) => {
      const t = e.touches[0];
      if (!t) return;
      cancel();
      const at = { x: t.clientX, y: t.clientY };
      press.current = { ...at, timer: window.setTimeout(() => setOpen({ ...at, label, entries: entries() }), LONG_PRESS_MS) };
    },
    onTouchMove: (e: TouchEvent) => {
      const t = e.touches[0];
      if (press.current && t && Math.hypot(t.clientX - press.current.x, t.clientY - press.current.y) > 10) cancel();
    },
    onTouchEnd: cancel,
  }), []);
  const menu = open ? <Menu menu={open} onClose={() => setOpen(null)} /> : null;
  return { bind, menu };
}
