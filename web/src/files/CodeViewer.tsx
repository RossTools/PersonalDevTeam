import { useEffect, useRef } from 'react';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers } from '@codemirror/view';
import { codeEditing, codeHighlight, codeTheme, languageFor } from './codeTheme';

/**
 * Code with line numbers, highlighted by the file's name (languages load on demand). Read-only unless `onChange` is
 * given: then `text` is where editing starts (later changes to it are ignored; remount with a new `key` to start over)
 * and every edit reports the whole text.
 */
export function CodeViewer({ name, text, onChange }: { name: string; text: string; onChange?: (text: string) => void }) {
  const host = useRef<HTMLDivElement>(null);
  const changed = useRef(onChange);
  changed.current = onChange;
  const editable = !!onChange;
  useEffect(() => {
    const language = new Compartment();
    // A file with Windows line endings keeps them (CodeMirror would otherwise save every line with `\n`).
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
                EditorView.updateListener.of((u) => { if (u.docChanged) changed.current?.(u.state.doc.toString()); }),
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
    void languageFor(name).then((support) => { if (alive && support) view.dispatch({ effects: language.reconfigure(support) }); });
    return () => {
      alive = false;
      view.destroy();
    };
    // While editing, `text` is only the starting point: the editor holds the text from then on.
  }, [name, editable, editable ? null : text]);
  return <div ref={host} data-testid="code-viewer" className="h-full min-h-0 overflow-hidden" />;
}
