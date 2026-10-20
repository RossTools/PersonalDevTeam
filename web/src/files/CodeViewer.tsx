import { useEffect, useRef } from 'react';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, lineNumbers } from '@codemirror/view';
import { codeHighlight, codeTheme, languageFor } from './codeTheme';

/** Read-only code with line numbers, highlighted by the file's name (languages load on demand). */
export function CodeViewer({ name, text }: { name: string; text: string }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const language = new Compartment();
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: text,
        extensions: [
          lineNumbers(),
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
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
  }, [name, text]);
  return <div ref={host} data-testid="code-viewer" className="h-full min-h-0 overflow-hidden" />;
}
