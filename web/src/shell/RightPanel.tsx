// The right panel (PDT-125): the user's todo list beside the work (Todos only since PDT-143; notes have a view of their
// own). Open, it is ~320px and can be dragged wider; collapsed, a 40px strip with the open todos' count. Desktop only:
// phones open the same list in a sheet from the top bar.
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { ListChecks, PanelRightClose } from 'lucide-react';
import { IconButton } from '@ui';
import type { Project } from '../api/gen/Project';
import { TodoList, type TodoFocus } from '../todos/TodoList';
import { useOpenTodoCount } from '../todos/useTodos';

export const RIGHT_PANEL_OPEN_KEY = 'pdt.rightPanel.open';
export const RIGHT_PANEL_WIDTH_KEY = 'pdt.rightPanel.width';
export const RIGHT_PANEL_WIDTH = { default: 320, min: 240, max: 640 };
const STEP = 16;

function read<T>(key: string, parse: (v: string | null) => T): T {
  try {
    return parse(localStorage.getItem(key));
  } catch {
    return parse(null);
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Private window or blocked storage: the panel keeps its state for this visit only.
  }
}

const clamp = (n: number) => Math.min(RIGHT_PANEL_WIDTH.max, Math.max(RIGHT_PANEL_WIDTH.min, n));

function readRightPanelOpen(): boolean {
  return read(RIGHT_PANEL_OPEN_KEY, (v) => v === 'true');
}

function readWidth(): number {
  return read(RIGHT_PANEL_WIDTH_KEY, (v) => {
    const n = Number(v);
    return v && Number.isFinite(n) && n > 0 ? clamp(n) : RIGHT_PANEL_WIDTH.default;
  });
}

/** Open/closed, kept per browser. */
export function useRightPanelOpen(): [boolean, (open: boolean | ((o: boolean) => boolean)) => void] {
  const [open, setOpen] = useState(readRightPanelOpen);
  const set = (next: boolean | ((o: boolean) => boolean)) =>
    setOpen((o) => {
      const v = typeof next === 'function' ? next(o) : next;
      write(RIGHT_PANEL_OPEN_KEY, String(v));
      return v;
    });
  return [open, set];
}

/** ⌘. (Mac) toggles the panel. */
export function useRightPanelKey(onToggle: () => void) {
  const handler = useRef(onToggle);
  handler.current = onToggle;
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if (e.key !== '.' && e.code !== 'Period') return;
      e.preventDefault();
      e.stopPropagation();
      handler.current();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);
}

/** The right panel's content (the todo list), shared by the desktop panel and the phone's sheet. */
export function RightPanelContent({ projectId, projects, focusTodo, onTodoFocused, focusAddSeq, trailing, onTodoOpen }: {
  projectId: string | null;
  projects: readonly Project[];
  /** A todo to show (a due reminder was opened). */
  focusTodo?: TodoFocus | null;
  /** The list has shown `focusTodo`: drop it, so reopening the panel doesn't show it again. */
  onTodoFocused?: () => void;
  /** Changes when the add box should take focus (opened with Ctrl+] t). */
  focusAddSeq?: number;
  /** At the end of the header (the desktop's collapse button). */
  trailing?: ReactNode;
  /** A todo was opened, or the list is back (a phone's sheet goes full screen for an open todo). */
  onTodoOpen?: (open: boolean) => void;
}) {
  return <TodoList projectId={projectId} projects={projects} focus={focusTodo} onFocused={onTodoFocused} focusAddSeq={focusAddSeq} trailing={trailing} onOpenChange={onTodoOpen} />;
}

export function RightPanel({ open, onOpenChange, projectId, projects, focusTodo, onTodoFocused, focusAddSeq }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The project on screen (the todo filter's "current project"). */
  projectId: string | null;
  projects: readonly Project[];
  focusTodo?: TodoFocus | null;
  onTodoFocused?: () => void;
  focusAddSeq?: number;
}) {
  const count = useOpenTodoCount();
  const [width, setWidth] = useState(readWidth);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const set = (n: number) => {
    const v = clamp(n);
    setWidth(v);
    write(RIGHT_PANEL_WIDTH_KEY, String(Math.round(v)));
  };
  const onKeyDown = (e: KeyboardEvent) => {
    // The divider is on the panel's left edge: Left widens it.
    const by = { ArrowLeft: STEP, ArrowRight: -STEP }[e.key];
    if (by) set(width + by);
    else if (e.key === 'Home') set(RIGHT_PANEL_WIDTH.max);
    else if (e.key === 'End') set(RIGHT_PANEL_WIDTH.min);
    else return;
    e.preventDefault();
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = { x: e.clientX, width };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent) => {
    if (drag.current) set(drag.current.width - (e.clientX - drag.current.x));
  };
  const onPointerUp = () => {
    drag.current = null;
  };

  if (!open) {
    const label = count > 0 ? `Open the todo list (${count} open)` : 'Open the todo list';
    return (
      <aside aria-label="Right panel" className="flex w-10 shrink-0 flex-col border-l border-border bg-card">
        <button
          type="button"
          aria-label={label}
          aria-expanded={false}
          title={`${label} (⌘.)`}
          onClick={() => onOpenChange(true)}
          className="flex flex-1 flex-col items-center gap-1 pt-2 text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
        >
          <ListChecks aria-hidden className="size-4" />
          {count > 0 && <span aria-hidden className="text-2xs font-medium tabular-nums">{count > 99 ? '99+' : count}</span>}
        </button>
      </aside>
    );
  }
  return (
    <aside aria-label="Right panel" className="flex min-h-0 shrink-0 border-l border-border bg-card" style={{ width }}>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the right panel"
        aria-valuenow={Math.round(width)}
        aria-valuemin={RIGHT_PANEL_WIDTH.min}
        aria-valuemax={RIGHT_PANEL_WIDTH.max}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        className="w-1 shrink-0 cursor-col-resize touch-none outline-none hover:bg-accent focus-visible:bg-primary"
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <RightPanelContent
          projectId={projectId}
          projects={projects}
          focusTodo={focusTodo}
          onTodoFocused={onTodoFocused}
          focusAddSeq={focusAddSeq}
          trailing={
            <IconButton label="Collapse the right panel" shortcut="⌘." aria-expanded size="sm" onClick={() => onOpenChange(false)} className="text-muted-foreground">
              <PanelRightClose aria-hidden />
            </IconButton>
          }
        />
      </div>
    </aside>
  );
}
