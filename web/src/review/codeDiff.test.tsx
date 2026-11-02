import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Chunk } from '@codemirror/merge';
import { Text } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { CodeDiff, oldToNewLine, wordChanges } from './CodeDiff';

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

describe('wordChanges', () => {
  const marked = (doc: Text, ranges: [number, number][]) => ranges.map(([f, t]) => doc.sliceString(f, t));

  it('marks only the words that differ in a line pair that is mostly the same', () => {
    const a = text('x\n  if (n < 8) return;\ny\n');
    const b = text('x\n  if (n < 12) return;\ny\n');
    const w = wordChanges(Chunk.build(a, b), a, b);
    expect(marked(a, w.a)).toEqual(['8']);
    expect(marked(b, w.b)).toEqual(['12']);
  });

  it('leaves wholly added lines and mostly different pairs unmarked', () => {
    const a = text('x\nconst res = await fetch(url);\ny\n');
    const b = text('x\nstatus.textContent = done;\ntry {\n  more();\n}\ny\n');
    const w = wordChanges(Chunk.build(a, b), a, b);
    expect(w.a).toEqual([]);
    expect(w.b).toEqual([]);
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

describe('CodeDiff collapsed runs and picked lines', () => {
  const lines = (n: number, change?: number) =>
    Array.from({ length: n }, (_, i) => (i + 1 === change ? `CHANGED ${i + 1}` : `line ${i + 1}`)).join('\n');

  for (const mode of ['split', 'unified'] as const) {
    it(`expands a collapsed run that holds a thread (${mode})`, async () => {
      const { container } = render(
        <CodeDiff
          path="a.txt"
          oldText={lines(40)}
          newText={lines(40, 2)}
          mode={mode}
          wholeFile={false}
          anchors={[{ id: 'deep', side: 'new', line: 30 }]}
          renderAnchor={(id) => <p>thread {id}</p>}
          onComment={vi.fn()}
        />,
      );
      await waitFor(() => expect(screen.getByText('thread deep')).toBeInTheDocument());
      // Only the run that held the thread opened: nothing else collapses (2 + 3 margin, then 6..40 is one run).
      expect(container.querySelectorAll('.cm-collapsedLines')).toHaveLength(0);
    });
  }

  it('keeps runs without threads collapsed', () => {
    const { container } = render(
      <CodeDiff
        path="a.txt"
        oldText={lines(40)}
        newText={lines(40, 20)}
        mode="unified"
        wholeFile={false}
        anchors={[{ id: 'near', side: 'new', line: 30 }]}
        renderAnchor={(id) => <p>thread {id}</p>}
        onComment={vi.fn()}
      />,
    );
    // Lines 1..16 collapse; 24..40 held the thread and opened.
    expect(container.querySelectorAll('.cm-collapsedLines')).toHaveLength(1);
  });

  it('picks lines in the number gutter and comments on them with c', () => {
    const onComment = vi.fn();
    const { container } = render(
      <CodeDiff
        path="a.txt"
        oldText={lines(6)}
        newText={lines(6, 2)}
        mode="unified"
        wholeFile
        anchors={[]}
        renderAnchor={() => null}
        onComment={onComment}
      />,
    );
    const gutter = container.querySelector('.cm-lineNumbers')!;
    const view = EditorView.findFromDOM(container.querySelector('.cm-editor') as HTMLElement)!;
    const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
    fireEvent.mouseDown(gutter, { clientY: y(3) });
    fireEvent.mouseDown(gutter, { clientY: y(5), shiftKey: true });
    expect(container.querySelectorAll('.cm-picked-line')).toHaveLength(3);
    fireEvent.keyDown(view.contentDOM, { key: 'c' });
    expect(onComment).toHaveBeenCalledWith({ side: 'new', line: 3, endLine: 5, lineText: 'line 3\nline 4\nline 5' });
    expect(container.querySelectorAll('.cm-picked-line')).toHaveLength(0);
  });

  it('on touch, a second tap extends the pick, "+" comments on it, and a tap inside drops it', () => {
    const onComment = vi.fn();
    const { container } = render(
      <CodeDiff path="a.txt" oldText={lines(6)} newText={lines(6, 2)} mode="unified" wholeFile anchors={[]} renderAnchor={() => null}
        onComment={onComment} touch />,
    );
    const numbers = container.querySelector('.cm-lineNumbers')!;
    const view = EditorView.findFromDOM(container.querySelector('.cm-editor') as HTMLElement)!;
    const y = (n: number) => view.documentTop + view.lineBlockAt(view.state.doc.line(n).from).top + 1;
    fireEvent.mouseDown(numbers, { clientY: y(3) });
    fireEvent.mouseDown(numbers, { clientY: y(5) });
    expect(container.querySelectorAll('.cm-picked-line')).toHaveLength(3);
    fireEvent.mouseDown(numbers, { clientY: y(4) });
    expect(container.querySelectorAll('.cm-picked-line')).toHaveLength(0);
    fireEvent.mouseDown(numbers, { clientY: y(2) });
    fireEvent.mouseDown(numbers, { clientY: y(3) });
    fireEvent.mouseDown(container.querySelector('.cm-comment-gutter')!, { clientY: y(3) });
    expect(onComment).toHaveBeenCalledWith({ side: 'new', line: 2, endLine: 3, lineText: 'CHANGED 2\nline 3' });
  });
});

describe('CodeDiff comment boxes', () => {
  // The read-only editor hides the browser's caret, and caret-color is inherited: a box under a line must reset it.
  for (const mode of ['split', 'unified'] as const) {
    it(`shows the caret in a box under a line (${mode})`, async () => {
      render(
        <CodeDiff
          path="a.ts"
          oldText={'one\ntwo\n'}
          newText={'one\nTWO\n'}
          mode={mode}
          wholeFile
          anchors={[{ id: 't1', side: 'new', line: 2 }, { id: 't2', side: 'old', line: 2 }]}
          renderAnchor={(id) => <textarea aria-label={`reply ${id}`} />}
          onComment={vi.fn()}
        />,
      );
      for (const id of ['t1', 't2']) {
        const box = await screen.findByLabelText(`reply ${id}`);
        const hidden = getComputedStyle(box.closest('.cm-content')!).caretColor;
        expect(hidden).toBe('rgba(0, 0, 0, 0)');
        expect(getComputedStyle(box).caretColor).not.toBe(hidden);
      }
    });
  }
});
