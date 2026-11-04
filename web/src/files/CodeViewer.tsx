import { useEffect, useRef } from 'react';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { Compartment, EditorState, StateEffect, StateField } from '@codemirror/state';
import { Decoration, EditorView, keymap, lineNumbers, type DecorationSet } from '@codemirror/view';
import { readViewState, useViewKey, writeViewState } from '../common/viewState';
import { codeEditing, codeHighlight, codeTheme, languageFor, v } from './codeTheme';

/** A place to show: a 1-based line and the text to mark in it (`[from, to)` in UTF-16 units); a new `n` shows it again. */
export interface CodeTarget {
  line: number;
  from: number;
  to: number;
  n: number;
}

const markHit = StateEffect.define<{ from: number; to: number }>();
const hitMark = Decoration.mark({ class: 'cm-pdt-hit' });
/** The marked match (a search result opened here). */
const hits = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    marks = marks.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(markHit)) marks = e.value.to > e.value.from ? Decoration.set([hitMark.range(e.value.from, e.value.to)]) : Decoration.none;
    }
    return marks;
  },
  provide: (f) => EditorView.decorations.from(f),
});
const hitTheme = EditorView.theme({ '.cm-pdt-hit': { backgroundColor: v('waiting-soft'), outline: `1px solid ${v('waiting')}`, borderRadius: '2px' } });

/** Scrolls `view` to `at`'s line (centred) and marks its match; an editor also selects it. */
function show(view: EditorView, at: CodeTarget, editable: boolean) {
  const doc = view.state.doc;
  const line = doc.line(Math.min(Math.max(at.line, 1), doc.lines));
  const from = Math.min(line.from + at.from, line.to);
  const to = Math.min(line.from + at.to, line.to);
  view.dispatch({
    effects: [markHit.of({ from, to }), EditorView.scrollIntoView(from, { y: 'center' })],
    ...(editable ? { selection: { anchor: from, head: to } } : {}),
  });
}

/**
 * Code with line numbers, highlighted by the file's name (languages load on demand). Read-only unless `onChange` is
 * given: then `text` is where editing starts (later changes to it are ignored; remount with a new `key` to start over)
 * and every edit reports the whole text. `remember` names where its scroll is kept (the view-state store), so it opens
 * where it was left; `at` opens it at a line instead, its match marked.
 */
export function CodeViewer({ name, text, onChange, remember, at }: { name: string; text: string; onChange?: (text: string) => void; remember?: string; at?: CodeTarget }) {
  const host = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const atRef = useRef(at);
  atRef.current = at;
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
          hits,
          hitTheme,
          language.of([]),
        ],
      }),
    });
    viewRef.current = view;
    let alive = true;
    const scroller = view.scrollDOM;
    const onScroll = () => writeViewState(scrollKey, scroller.scrollTop);
    if (atRef.current) show(view, atRef.current, editable);
    if (remember) {
      const top = atRef.current ? undefined : readViewState<number>(scrollKey);
      // Once CodeMirror has measured its lines.
      if (typeof top === 'number') requestAnimationFrame(() => { if (alive) scroller.scrollTop = top; });
      scroller.addEventListener('scroll', onScroll, { passive: true });
    }
    void languageFor(name).then((support) => { if (alive && support) view.dispatch({ effects: language.reconfigure(support) }); });
    return () => {
      alive = false;
      viewRef.current = null;
      scroller.removeEventListener('scroll', onScroll);
      view.destroy();
    };
    // While editing, `text` is only the starting point: the editor holds the text from then on.
  }, [name, editable, editable ? null : text]);
  // Another result opened in the file shown: go to it.
  const shown = useRef(at?.n);
  useEffect(() => {
    if (!at || shown.current === at.n) return;
    shown.current = at.n;
    if (viewRef.current) show(viewRef.current, at, editable);
  }, [at?.n]);
  return <div ref={host} data-testid="code-viewer" className="h-full min-h-0 overflow-hidden" />;
}
