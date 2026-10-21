import { useEffect, useRef } from 'react';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers } from '@codemirror/view';
import { codeHighlight, codeTheme, languageFor } from '../files/codeTheme';

/**
 * Editable code (CodeMirror) highlighted by `name`'s extension. Controlled loosely: `value` replaces the text only when
 * it differs from what the editor holds, so typing is never interrupted by the echo of its own change.
 */
export function CodeEditor({ name, value, onChange, readOnly = false, label }: {
  name: string; value: string; onChange: (text: string) => void; readOnly?: boolean; label: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const change = useRef(onChange);
  change.current = onChange;
  useEffect(() => {
    const language = new Compartment();
    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          EditorView.lineWrapping,
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
          EditorView.contentAttributes.of({ 'aria-label': label }),
          EditorView.updateListener.of((u) => { if (u.docChanged) change.current(u.state.doc.toString()); }),
          codeTheme,
          codeHighlight,
          language.of([]),
        ],
      }),
    });
    view.current = v;
    let alive = true;
    void languageFor(name).then((support) => { if (alive && support) v.dispatch({ effects: language.reconfigure(support) }); });
    return () => {
      alive = false;
      v.destroy();
      view.current = null;
    };
    // The text follows `value` below; a new name or mode makes a new editor.
  }, [name, readOnly, label]);
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value]);
  return <div ref={host} data-testid="code-editor" className="h-full min-h-0 overflow-auto" />;
}
