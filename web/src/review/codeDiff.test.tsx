import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Chunk } from '@codemirror/merge';
import { Text } from '@codemirror/state';
import { CodeDiff, oldToNewLine } from './CodeDiff';

const text = (s: string) => Text.of(s.split('\n'));

describe('oldToNewLine', () => {
  it('maps unchanged, removed and trailing lines of the old text into the new', () => {
    const a = text('a\nb\nc\nd\n');
    const b = text('a\nX\nc\nd\ne\n');
    const chunks = Chunk.build(a, b);
    expect(oldToNewLine(chunks, a, b, 1)).toBe(1);
    expect(oldToNewLine(chunks, a, b, 2)).toBe(2); // removed "b": beside the "X" that replaced it
    expect(oldToNewLine(chunks, a, b, 4)).toBe(4);
    const gone = text('a\nb\nc');
    const kept = text('a\nc');
    expect(oldToNewLine(Chunk.build(gone, kept), gone, kept, 2)).toBe(1); // a pure removal: after the line before it
    expect(oldToNewLine(Chunk.build(gone, kept), gone, kept, 3)).toBe(2);
  });
});

describe('CodeDiff', () => {
  for (const mode of ['split', 'unified'] as const) {
    it(`draws a ${mode} diff with threads under their lines`, async () => {
      render(
        <CodeDiff
          path="a.ts"
          oldText={'one\ntwo\nthree\n'}
          newText={'one\nTWO\nthree\nfour\n'}
          mode={mode}
          wholeFile
          anchors={[{ id: 't1', side: 'new', line: 2 }, { id: 't2', side: 'old', line: 2 }]}
          renderAnchor={(id) => <p>thread {id}</p>}
          onComment={vi.fn()}
        />,
      );
      expect(screen.getByTestId('code-diff')).toBeInTheDocument();
      await waitFor(() => expect(screen.getByText('thread t1')).toBeInTheDocument());
      expect(screen.getByText('thread t2')).toBeInTheDocument();
      expect(screen.getAllByText('TWO').length).toBeGreaterThan(0);
    });
  }
});
