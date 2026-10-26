import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { CodeViewer } from './CodeViewer';
import { CodeEditor } from '../workflows/CodeEditor';

// Editable editors draw their own cursor (drawSelection's cursor layer) and mark the active line; read-only views do neither.
const cursor = (el: HTMLElement) => el.querySelector('.cm-cursorLayer');
const activeLine = (el: HTMLElement) => el.querySelector('.cm-activeLine');

describe('code editing cursor', () => {
  it('an editable file shows a cursor and the active line', () => {
    const { container } = render(<CodeViewer name="a.txt" text={'one\ntwo'} onChange={() => {}} />);
    expect(cursor(container)).not.toBeNull();
    expect(activeLine(container)).not.toBeNull();
  });

  it('a read-only file view has no cursor or active line', () => {
    const { container } = render(<CodeViewer name="a.txt" text={'one\ntwo'} />);
    expect(cursor(container)).toBeNull();
    expect(activeLine(container)).toBeNull();
  });

  it('the workflow code editor shows a cursor and the active line, unless read-only', () => {
    const editable = render(<CodeEditor name="w.ts" value="x" onChange={() => {}} label="Code" />);
    expect(cursor(editable.container)).not.toBeNull();
    expect(activeLine(editable.container)).not.toBeNull();
    const readOnly = render(<CodeEditor name="w.ts" value="x" onChange={() => {}} readOnly label="Code" />);
    expect(cursor(readOnly.container)).toBeNull();
  });
});
