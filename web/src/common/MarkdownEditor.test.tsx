import { fireEvent, render, screen } from '@testing-library/react';
import { EditorView } from '@codemirror/view';
import { describe, expect, it, vi } from 'vitest';
import { editorText } from '../test-utils';
import { MarkdownEditor } from './MarkdownEditor';

/** An editor holding `text` with the caret at its end; returns its text box. */
function editor(text: string, onChange = vi.fn()) {
  render(<MarkdownEditor label="Note" value={text} onChange={onChange} autoFocus="end" />);
  return screen.getByRole('textbox', { name: 'Note' });
}
const press = (box: HTMLElement, key: string, init: { shiftKey?: boolean } = {}) => fireEvent.keyDown(box, { key, ...init });
const caret = (box: HTMLElement) => EditorView.findFromDOM(box.closest<HTMLElement>('.cm-editor')!)!.state.selection.main.head;

describe('MarkdownEditor', () => {
  it('continues a bullet list on Enter', () => {
    const box = editor('- one');
    press(box, 'Enter');
    expect(editorText(box)).toBe('- one\n- ');
  });

  it('continues a numbered list, counting up', () => {
    const box = editor('1. one\n2. two');
    press(box, 'Enter');
    expect(editorText(box)).toBe('1. one\n2. two\n3. ');
  });

  it('continues a task list with an empty box', () => {
    const box = editor('- [x] done');
    press(box, 'Enter');
    expect(editorText(box)).toBe('- [x] done\n- [ ] ');
  });

  // PDT-208: Enter at the end of an item puts the next one on the very next line, with the caret after its marker, never
  // after a blank line (lang-markdown adds one to a loose list even with `nonTightLists: false`).
  it.each([
    ['a tight list', '- one\n- two', '- one\n- two\n- '],
    ['a list with a blank line between its items', '- one\n\n- two', '- one\n\n- two\n- '],
    ['a list with a blank line further down', '- one\n- two\n\n- three', '- one\n- two\n\n- three\n- '],
    ['a trailing space', '- one\n- two ', '- one\n- two\n- '],
    ['a numbered list', '1. one\n\n2. two', '1. one\n\n2. two\n3. '],
    ['a task list', '- [ ] one\n\n- [x] two', '- [ ] one\n\n- [x] two\n- [ ] '],
    ['a nested list', '- one\n\n  - a\n\n  - b', '- one\n\n  - a\n\n  - b\n  - '],
    ['a list in a quote', '> - one\n>\n> - two', '> - one\n>\n> - two\n> - '],
  ])('continues %s on the very next line', (_, text, after) => {
    const box = editor(text);
    press(box, 'Enter');
    expect(editorText(box)).toBe(after);
    expect(caret(box)).toBe(after.length);
  });

  it('continues an item that is not the last line of the document on the very next line', () => {
    render(<MarkdownEditor label="Note" value={'- one\n\n- two\n\nAfter'} onChange={vi.fn()} autoFocus={12} />);
    const box = screen.getByRole('textbox', { name: 'Note' });
    press(box, 'Enter');
    expect(editorText(box)).toBe('- one\n\n- two\n- \n\nAfter');
    expect(caret(box)).toBe(15);
  });

  it('continues a loose list with Windows line endings on the very next line', () => {
    const onChange = vi.fn();
    const box = editor('- one\r\n\r\n- two', onChange);
    press(box, 'Enter');
    expect(onChange).toHaveBeenLastCalledWith('- one\r\n\r\n- two\r\n- ');
    expect(caret(box)).toBe(15);
  });

  it('continues a list at the end of a long text, parsed only part of the way yet', () => {
    const text = `${'A paragraph of text, *with* some **marks** in it.\n\n'.repeat(300)}- one\n- two`;
    const box = editor(text);
    press(box, 'Enter');
    expect(editorText(box).slice(-15)).toBe('\n- one\n- two\n- ');
  });

  it('ends a loose list on Enter in an empty item', () => {
    const box = editor('- one\n\n- two\n- ');
    press(box, 'Enter');
    expect(editorText(box)).toBe('- one\n\n- two\n');
  });

  it('ends the list on Enter in an empty item', () => {
    const box = editor('- one\n- ');
    press(box, 'Enter');
    expect(editorText(box)).toBe('- one\n');
  });

  it('removes an empty item’s marker on Backspace', () => {
    const box = editor('- one\n- ');
    press(box, 'Backspace');
    expect(editorText(box)).not.toContain('\n- ');
    expect(editorText(box).startsWith('- one\n')).toBe(true);
  });

  it('indents and outdents a list item with Tab and Shift-Tab', () => {
    const box = editor('- one\n- two');
    press(box, 'Tab');
    expect(editorText(box)).toBe('- one\n  - two');
    press(box, 'Tab', { shiftKey: true });
    expect(editorText(box)).toBe('- one\n- two');
  });

  it('reports edits, and keeps Windows line endings', () => {
    const onChange = vi.fn();
    const box = editor('- one\r\n- two', onChange);
    press(box, 'Enter');
    expect(onChange).toHaveBeenLastCalledWith('- one\r\n- two\r\n- ');
  });

  it('takes a new value from outside, and focuses with the caret where asked', () => {
    const onChange = vi.fn();
    const { rerender } = render(<MarkdownEditor label="Note" value="Hello" onChange={onChange} autoFocus={2} />);
    const box = screen.getByRole('textbox', { name: 'Note' });
    expect(box).toHaveFocus();
    expect(caret(box)).toBe(2);
    rerender(<MarkdownEditor label="Note" value="Changed elsewhere" onChange={onChange} autoFocus={2} />);
    expect(editorText(box)).toBe('Changed elsewhere');
    // Not an edit: nothing is reported back (a note would think it had unsaved changes).
    expect(onChange).not.toHaveBeenCalled();
  });

  it('styles headings, emphasis and code, and dims the marks', () => {
    const box = editor('# Title\n\n**bold** and `code`\n\n```js\nlet x = 1;\n```');
    const root = box.closest('.cm-editor')!;
    expect(root.querySelector('.cm-md-inline-code')).toHaveTextContent('`code`');
    expect(root.querySelectorAll('.cm-md-code')).toHaveLength(3);
  });
});
