import { useEffect, useRef } from 'react';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers } from '@codemirror/view';
import { readViewState, useViewKey, writeViewState } from '../common/viewState';
import { codeEditing, codeHighlight, codeTheme, languageFor } from './codeTheme';

/**
 * Code with line numbers, highlighted by the file's name (languages load on demand). Read-only unless `onChange` is
 * given: then `text` is where editing starts (later changes to it are ignored; remount with a new `key` to start over)
 * and every edit reports the whole text. `remember` names where its scroll is kept (the view-state store), so it opens
 * where it was left.
 */
export function CodeViewer({ name, text, onChange, remember }: { name: string; text: string; onChange?: (text: string) => void; remember?: string }) {
  const host = useRef<HTMLDivElement>(null);
  const scrollKey = useViewKey(remember ?? '');
  const changed = useRef(onChange);
  changed.current = onChange;
  const editable = !!onChange;
  useEffect(() => {
    const language = new Compartment();
    // A file with Windows line endings keeps them (CodeMirror would otherwise save every line with `\n`): the text is read
    // with sliceDoc, which joins lines with the separator (doc.toString always uses `\n`).
    const crlf = editable && text.includes('\r\n');
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: text,
        extensions: [
          lineNumbers(),
          ...(editable
            ? [
                codeEditing,
                history(),
                keymap.of([...defaultKeymap, ...historyKeymap]),
                ...(crlf ? [EditorState.lineSeparator.of('\r\n')] : []),
                EditorView.updateListener.of((u) => { if (u.docChanged) changed.current?.(u.state.sliceDoc()); }),
              ]
            : [EditorState.readOnly.of(true), EditorView.editable.of(false)]),
          EditorView.lineWrapping,
          codeTheme,
          codeHighlight,
          language.of([]),
        ],
      }),
    });
    let alive = true;
    const scroller = view.scrollDOM;
    const onScroll = () => writeViewState(scrollKey, scroller.scrollTop);
    if (remember) {
      const top = readViewState<number>(scrollKey);
      // Once CodeMirror has measured its lines.
      if (typeof top === 'number') requestAnimationFrame(() => { if (alive) scroller.scrollTop = top; });
      scroller.addEventListener('scroll', onScroll, { passive: true });
    }
    void languageFor(name).then((support) => { if (alive && support) view.dispatch({ effects: language.reconfigure(support) }); });
    return () => {
      alive = false;
      scroller.removeEventListener('scroll', onScroll);
      view.destroy();
    };
    // While editing, `text` is only the starting point: the editor holds the text from then on.
  }, [name, editable, editable ? null : text]);
  return <div ref={host} data-testid="code-viewer" className="h-full min-h-0 overflow-hidden" />;
}
