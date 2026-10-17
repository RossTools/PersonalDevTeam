import { describe, expect, it } from 'vitest';
import { linkSegments, type BufferLike } from './linkSpan';

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
  it('finds the parts of the same link on the rows above and below, not the hovered row', () => {
    const b = buffer([
      'text  ',
      'see 11',
      '  1111',
      '  11 .',
      '  22  ',
    ]);
    expect(linkSegments(b, 2, 3)).toEqual([
      { row: 1, start: 4, end: 6 },
      { row: 3, start: 2, end: 4 },
    ]);
  });

  it('returns nothing for a cell without a link or a link on one row', () => {
    const b = buffer(['  11  ', '  33  ']);
    expect(linkSegments(b, 0, 0)).toEqual([]);
    expect(linkSegments(b, 0, 2)).toEqual([]);
  });

  it('works without the internal cell field', () => {
    const b: BufferLike = { getLine: () => ({ length: 3, getCell: () => ({}) }) };
    expect(linkSegments(b, 0, 1)).toEqual([]);
  });
});
