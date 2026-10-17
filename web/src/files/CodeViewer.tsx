import { useEffect, useRef } from 'react';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, lineNumbers } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { LanguageDescription } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';

// Colours come from the design tokens (tokens.css), so the viewer follows light and dark mode.
const v = (name: string) => `var(--pdt-${name})`;

const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: v('bg'), color: v('fg'), fontSize: '12.5px' },
  '.cm-scroller': { fontFamily: v('font-mono'), lineHeight: '1.5' },
  '.cm-gutters': { backgroundColor: v('surface'), color: v('muted'), borderRight: `1px solid ${v('border')}` },
  '.cm-content': { caretColor: 'transparent' },
  '&.cm-focused': { outline: 'none' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: v('accent-soft') },
});

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], color: v('term-magenta') },
  { tag: [t.string, t.special(t.string), t.regexp], color: v('term-green') },
  { tag: [t.number, t.bool, t.null, t.atom], color: v('term-yellow') },
  { tag: [t.comment, t.lineComment, t.blockComment], color: v('muted'), fontStyle: 'italic' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: v('term-blue') },
  { tag: [t.typeName, t.className, t.namespace], color: v('term-cyan') },
  { tag: [t.propertyName, t.attributeName], color: v('term-red') },
  { tag: [t.heading, t.strong], fontWeight: 'bold' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.link, textDecoration: 'underline' },
  { tag: t.invalid, color: v('failed') },
]);

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
          theme,
          syntaxHighlighting(highlight),
          language.of([]),
        ],
      }),
    });
    let alive = true;
    const desc = LanguageDescription.matchFilename(languages, name);
    desc?.load().then((support) => { if (alive) view.dispatch({ effects: language.reconfigure(support) }); }, () => {});
    return () => {
      alive = false;
      view.destroy();
    };
  }, [name, text]);
  return <div ref={host} data-testid="code-viewer" className="h-full min-h-0 overflow-hidden" />;
}
