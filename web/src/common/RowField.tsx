import type { ReactNode } from 'react';

const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

/** Keeps keys and clicks inside an in-row text field away from the row (typeahead, selection, drag). */
export function RowField({ children }: { children: ReactNode }) {
  return (
    <span className="block" onKeyDown={stop} onKeyUp={stop} onPointerDown={stop} onMouseDown={stop} onClick={stop} onDoubleClick={stop}>
      {children}
    </span>
  );
}
