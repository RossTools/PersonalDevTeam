import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { tags as t } from '@lezer/highlight';

// Colours come from the design tokens (tokens.css), so the viewers follow light and dark mode.
export const v = (name: string) => `var(--pdt-${name})`;

export const codeTheme = EditorView.theme({
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

export const codeHighlight: Extension = syntaxHighlighting(highlight);

/** The language support for a file name, loaded on demand (null when none matches or it fails to load). */
export async function languageFor(name: string): Promise<Extension | null> {
  const desc = LanguageDescription.matchFilename(languages, name);
  if (!desc) return null;
  try {
    return await desc.load();
  } catch {
    return null;
  }
}
