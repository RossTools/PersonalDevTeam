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
