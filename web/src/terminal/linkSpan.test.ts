import { describe, expect, it, vi } from 'vitest';
import { Terminal } from '@xterm/xterm';
import { linkSegments, linkUnderliner, type BufferLike } from './linkSpan';

/** Rows of cells; a digit is a cell of that OSC 8 link id, anything else has no link. */
function buffer(rows: string[]): BufferLike {
  return {
    getLine: (y) =>
      rows[y] === undefined
        ? undefined
        : {
            length: rows[y].length,
            getCell: (x) => {
              const ch = rows[y][x];
              return ch === undefined ? undefined : { extended: { urlId: /\d/.test(ch) ? Number(ch) : 0 } };
            },
          },
  };
}

describe('linkSegments', () => {
  it('finds every row of the same link, the hovered one included', () => {
    const b = buffer([
      'text  ',
      'see 11',
      '  1111',
      '  11 .',
      '  22  ',
    ]);
    expect(linkSegments(b, 2, 3)).toEqual([
      { row: 1, start: 4, end: 6 },
      { row: 2, start: 2, end: 6 },
      { row: 3, start: 2, end: 4 },
    ]);
  });

  it('returns nothing for a cell without a link, and one row for a link on one row', () => {
    const b = buffer(['  11  ', '  33  ']);
    expect(linkSegments(b, 0, 0)).toEqual([]);
    expect(linkSegments(b, 0, 2)).toEqual([{ row: 0, start: 2, end: 4 }]);
  });

  it('works without the internal cell field', () => {
    const b: BufferLike = { getLine: () => ({ length: 3, getCell: () => ({}) }) };
    expect(linkSegments(b, 0, 1)).toEqual([]);
  });
});

describe('linkUnderliner on a real xterm', () => {
  const write = (term: Terminal, data: string) => new Promise<void>((r) => term.write(data, r));
  // How Claude Code prints a URL that wraps: one OSC 8 link per row, all with the same id.
  const link = (text: string) => `\x1b]8;id=a1;https://example.com/long\x07${text}\x1b]8;;\x07`;

  it('underlines every row of a wrapped link, the hovered one included', async () => {
    const term = new Terminal({ cols: 20, rows: 5, allowProposedApi: true });
    await write(term, `see ${link('https://exa')}\r\n  ${link('mple.com/long')}\r\n`);
    const register = vi.spyOn(term, 'registerDecoration');
    const u = linkUnderliner(term);
    u.show(5, 1); // hovering row 1 (1-based), where the link starts at column 5
    expect(register).toHaveBeenCalledTimes(2);
    expect(register.mock.calls[0][0]).toMatchObject({ x: 4, width: 11 });
    expect(register.mock.calls[1][0]).toMatchObject({ x: 2, width: 13 });
    expect(register.mock.results.every((r) => r.value !== undefined)).toBe(true);
    u.dispose();
  });

  it('leaves a link on one row to xterm', async () => {
    const term = new Terminal({ cols: 20, rows: 5, allowProposedApi: true });
    await write(term, `see ${link('short')}\r\n`);
    const register = vi.spyOn(term, 'registerDecoration');
    linkUnderliner(term).show(5, 1);
    expect(register).not.toHaveBeenCalled();
  });

  it('does nothing, without throwing, when proposed APIs are off', async () => {
    const term = new Terminal({ cols: 20, rows: 5 });
    await write(term, `${link('https://exa')}\r\n${link('mple.com')}\r\n`);
    expect(() => linkUnderliner(term).show(1, 1)).not.toThrow();
  });
});
