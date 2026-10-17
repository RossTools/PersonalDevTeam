import type { IDisposable, Terminal } from '@xterm/xterm';
import { cssVar } from './theme';

/** The part of xterm's buffer API used here. `extended.urlId` is xterm's (undocumented) OSC 8 link id of a cell. */
export interface BufferLike {
  getLine(y: number): { length: number; getCell(x: number): { extended?: { urlId?: number } } | undefined } | undefined;
}

export interface Segment {
  /** 0-based buffer row. */
  row: number;
  start: number;
  /** Exclusive. */
  end: number;
}

const urlIdAt = (b: BufferLike, row: number, x: number) => b.getLine(row)?.getCell(x)?.extended?.urlId ?? 0;

function segmentOn(b: BufferLike, row: number, id: number): Segment | null {
  const line = b.getLine(row);
  if (!line) return null;
  let start = -1;
  let end = -1;
  for (let x = 0; x < line.length; x++) {
    if (urlIdAt(b, row, x) === id) {
      if (start < 0) start = x;
      end = x + 1;
    }
  }
  return start < 0 ? null : { row, start, end };
}

/**
 * Every row's part of the OSC 8 link at (row, x), top to bottom: an agent prints a URL that wraps as one link per
 * row with the same id. xterm underlines only the hovered row's part.
 */
export function linkSegments(b: BufferLike, row: number, x: number): Segment[] {
  const id = urlIdAt(b, row, x);
  const here = id ? segmentOn(b, row, id) : null;
  if (!here) return [];
  const out: Segment[] = [here];
  for (let r = row - 1; ; r--) {
    const s = segmentOn(b, r, id);
    if (!s) break;
    out.unshift(s);
  }
  for (let r = row + 1; ; r++) {
    const s = segmentOn(b, r, id);
    if (!s) break;
    out.push(s);
  }
  return out;
}

/**
 * Underlines every row of a wrapped OSC 8 link while one of its rows is hovered (call from the link handler's hover
 * and leave). The hovered row is drawn too: adding decorations re-renders the rows, which wipes xterm's own hover
 * underline. A link on one row is left to xterm. Normal buffer only: decorations need markers, which the alternate
 * screen doesn't have.
 */
export function linkUnderliner(term: Terminal): { show(x: number, y: number): void; clear(): void } & IDisposable {
  let shown: IDisposable[] = [];
  const clear = () => {
    shown.forEach((d) => d.dispose());
    shown = [];
  };
  return {
    // x and y are xterm's 1-based buffer range coordinates.
    show(x, y) {
      clear();
      try {
        draw(x, y);
      } catch {
        // Decorations are a proposed xterm API; without them only the hovered row is underlined.
        clear();
      }
    },
    clear,
    dispose: clear,
  };

  function draw(x: number, y: number) {
    const buf = term.buffer.active;
    if (buf.type !== 'normal') return;
    const cursorRow = buf.baseY + buf.cursorY;
    const segments = linkSegments(buf as unknown as BufferLike, y - 1, x - 1);
    if (segments.length < 2) return;
    for (const s of segments) {
      const marker = term.registerMarker(s.row - cursorRow);
      const deco = term.registerDecoration({ marker, x: s.start, width: s.end - s.start, layer: 'top' });
      if (!deco) {
        marker.dispose();
        continue;
      }
      deco.onRender((el) => {
        el.style.boxSizing = 'border-box';
        el.style.borderBottom = `1px solid ${cssVar('--pdt-fg')}`;
        el.style.pointerEvents = 'none';
      });
      shown.push(deco, marker);
    }
  }
}
