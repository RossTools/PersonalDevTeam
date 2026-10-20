import { useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { cn } from './lib/utils';

export interface SplitPaneProps {
  /** Remembers the size per browser under this id. */
  id: string;
  /** The left pane, whose width the divider sets. */
  start: ReactNode;
  end: ReactNode;
  /** The start pane's width in px. */
  defaultSize?: number;
  min?: number;
  max?: number;
  /** The divider's accessible name. */
  label?: string;
  className?: string;
}

const STEP = 16;
const key = (id: string) => `pdt.split.${id}`;

function stored(id: string): number | null {
  try {
    const n = Number(localStorage.getItem(key(id)));
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

function store(id: string, n: number) {
  try {
    localStorage.setItem(key(id), String(Math.round(n)));
  } catch {
    // Storage blocked: the size lasts for this page.
  }
}

/**
 * Two panes side by side with a draggable divider (a `separator` that also moves with Left/Right, Home/End). The start
 * pane's width is kept per browser.
 */
export function SplitPane({ id, start, end, defaultSize = 280, min = 160, max = 640, label = 'Resize', className }: SplitPaneProps) {
  const clamp = (n: number) => Math.min(max, Math.max(min, n));
  const [size, setSize] = useState(() => clamp(stored(id) ?? defaultSize));
  const drag = useRef<{ x: number; size: number } | null>(null);
  const set = (n: number) => {
    const v = clamp(n);
    setSize(v);
    store(id, v);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const by = { ArrowLeft: -STEP, ArrowRight: STEP }[e.key];
    if (by) set(size + by);
    else if (e.key === 'Home') set(min);
    else if (e.key === 'End') set(max);
    else return;
    e.preventDefault();
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = { x: e.clientX, size };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent) => {
    if (drag.current) set(drag.current.size + e.clientX - drag.current.x);
  };
  const onPointerUp = () => {
    drag.current = null;
  };
  return (
    <div className={cn('flex min-h-0 min-w-0 flex-1', className)}>
      <div className="flex min-h-0 shrink-0 flex-col overflow-hidden" style={{ width: size }}>
        {start}
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={label}
        aria-valuenow={Math.round(size)}
        aria-valuemin={min}
        aria-valuemax={max}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        className="w-1 shrink-0 cursor-col-resize touch-none border-l border-border outline-none hover:bg-accent focus-visible:bg-primary"
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">{end}</div>
    </div>
  );
}
