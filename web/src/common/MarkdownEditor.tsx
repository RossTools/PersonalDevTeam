import { useEffect, useImperativeHandle, useRef, type KeyboardEvent, type Ref } from 'react';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { deleteMarkupBackward, insertNewlineContinueMarkupCommand, markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { HighlightStyle, syntaxHighlighting, syntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { Annotation, EditorState, Prec, RangeSetBuilder, Transaction, type Extension } from '@codemirror/state';
import {
  Decoration, EditorView, ViewPlugin, drawSelection, keymap, lineNumbers, placeholder as placeholderText,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import { tags as t } from '@lezer/highlight';
import { cn } from '@ui/lib/utils';
import { codeEditing, codeHighlight, v } from '../files/codeTheme';

/** Moves the caret into the editor: to a position in the text, to its end, or (no argument) where it was. */
export interface MarkdownEditorHandle {
  focus(at?: number | 'end'): void;
}

export interface MarkdownEditorProps {
  /** The text. Followed loosely: it replaces the editor's text only when it differs, so typing is never interrupted
   *  by the echo of its own change. */
  value: string;
  onChange: (text: string) => void;
  /** The text box's accessible name. */
  label: string;
  placeholder?: string;
  /** `note`: a page of prose (notes), no gutter; `file`: a Markdown file, with line numbers and the active line. */
  variant?: 'note' | 'file';
  /** Focus the editor when it opens, with the caret here. */
  autoFocus?: number | 'end';
  onBlur?: () => void;
  onKeyDown?: (e: KeyboardEvent) => void;
  className?: string;
  ref?: Ref<MarkdownEditorHandle>;
}

/**
 * Markdown as its source, styled as you type (PDT-187): headings, emphasis, code, links and quotes look like what they
 * make, while `#`, `**`, `-` and the other marks stay visible, dimmed. Enter continues a list or quote (an empty item
 * ends it), Backspace after a list marker removes it, and Tab / Shift-Tab indent and outdent. A text with Windows line
 * endings keeps them.
 */
export function MarkdownEditor({ value, onChange, label, placeholder, variant = 'note', autoFocus, onBlur, onKeyDown, className, ref }: MarkdownEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const change = useRef(onChange);
  change.current = onChange;
  const start = useRef({ value, autoFocus });

  useEffect(() => {
    const { value: doc, autoFocus: focusAt } = start.current;
    const crlf = doc.includes('\r\n');
    const ed = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc,
        extensions: [
          ...(variant === 'file' ? [lineNumbers(), codeEditing] : [drawSelection()]),
          ...(crlf ? [EditorState.lineSeparator.of('\r\n')] : []),
          EditorView.lineWrapping,
          history(),
          markdown({ base: markdownLanguage, codeLanguages: languages, addKeymap: false }),
          Prec.high(listKeys),
          keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
          EditorView.contentAttributes.of({ 'aria-label': label }),
          ...(placeholder ? [placeholderText(placeholder)] : []),
          EditorView.updateListener.of((u) => {
            if (u.docChanged && !u.transactions.some((tr) => tr.annotation(fromValue))) change.current(u.state.sliceDoc());
          }),
          markdownTheme(variant),
          blockDecorations,
          syntaxHighlighting(markdownHighlight),
          codeHighlight,
        ],
      }),
    });
    view.current = ed;
    if (focusAt !== undefined) focusEditor(ed, focusAt);
    return () => {
      ed.destroy();
      view.current = null;
    };
  }, [variant, label, placeholder]);

  useEffect(() => {
    const ed = view.current;
    if (ed && ed.state.sliceDoc() !== value) {
      ed.dispatch({ changes: { from: 0, to: ed.state.doc.length, insert: value }, annotations: [fromValue.of(true), Transaction.addToHistory.of(false)] });
    }
  }, [value]);

  useImperativeHandle(ref, () => ({ focus: (at) => { if (view.current) focusEditor(view.current, at); } }), []);

  return (
    <div
      ref={host}
      data-testid="markdown-editor"
      onBlur={onBlur}
      onKeyDown={onKeyDown}
      className={cn('min-h-0', variant === 'file' && 'h-full overflow-hidden', className)}
    />
  );
}

// Enter continues a list or quote and, on an empty item, ends it (rather than starting a loose list); Backspace after a
// list marker removes it.
const listKeys = keymap.of([
  { key: 'Enter', run: insertNewlineContinueMarkupCommand({ nonTightLists: false }) },
  { key: 'Backspace', run: deleteMarkupBackward },
]);

/** Marks a change that came in through `value` (not an edit, so not reported back). */
const fromValue = Annotation.define<boolean>();

function focusEditor(ed: EditorView, at?: number | 'end') {
  if (at !== undefined) {
    const pos = at === 'end' ? ed.state.doc.length : Math.max(0, Math.min(at, ed.state.doc.length));
    ed.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
  }
  ed.focus();
}

// Sizes follow the `document` style of Markdown.tsx: H1–H3 bold at the body's size, H4 a step down, H5–H6 smaller and
// muted. In em, so they scale with the editor's own size.
const markdownHighlight = HighlightStyle.define([
  { tag: [t.heading1, t.heading2, t.heading3], fontWeight: '600', letterSpacing: '-0.01em' },
  { tag: t.heading4, fontWeight: '600', fontSize: '0.875em' },
  { tag: [t.heading5, t.heading6], fontWeight: '600', fontSize: '0.75em', color: v('muted') },
  { tag: t.strong, fontWeight: '600' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.strikethrough, textDecoration: 'line-through', color: v('muted') },
  { tag: t.monospace, fontFamily: v('font-mono'), fontSize: '0.875em' },
  { tag: t.link, color: v('accent') },
  { tag: t.url, color: v('muted'), textDecoration: 'underline' },
  { tag: t.quote, color: v('muted') },
  // The marks themselves (`#`, `**`, `-`, `>`, backticks, brackets) stay, dimmed.
  { tag: [t.processingInstruction, t.labelName, t.contentSeparator], color: v('muted') },
  { tag: t.atom, color: v('muted') },
]);

function markdownTheme(variant: 'note' | 'file'): Extension {
  const note = variant === 'note';
  return EditorView.theme({
    '&': {
      backgroundColor: note ? 'transparent' : v('bg'),
      color: v('fg'),
      fontSize: note ? '1rem' : '0.875rem',
      ...(note ? {} : { height: '100%' }),
    },
    '&.cm-focused': { outline: 'none' },
    '.cm-scroller': { fontFamily: v('font-sans'), lineHeight: note ? '1.7' : '1.6' },
    '.cm-content': { padding: note ? '0' : '12px 0', caretColor: v('fg'), ...(note ? { minHeight: '10rem' } : {}) },
    '.cm-line': { padding: note ? '0' : '0 12px' },
    '.cm-gutters': { backgroundColor: v('surface'), color: v('muted'), borderRight: `1px solid ${v('border')}`, fontFamily: v('font-mono') },
    '.cm-placeholder': { color: v('muted') },
    '&.cm-editor .cm-cursor, &.cm-editor .cm-dropCursor': { borderLeftColor: v('fg'), borderLeftWidth: '2px' },
    '&.cm-editor .cm-selectionBackground, &.cm-editor.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': {
      backgroundColor: v('accent-soft'),
    },
    '.cm-md-code': { backgroundColor: v('surface-2'), fontFamily: v('font-mono'), fontSize: '0.875em' },
    '.cm-md-inline-code': { backgroundColor: v('surface-2'), borderRadius: '3px', padding: '1px 2px' },
    '.cm-md-quote': { borderLeft: `3px solid ${v('border')}`, paddingLeft: note ? '12px' : '20px' },
  });
}

const codeLine = Decoration.line({ class: 'cm-md-code' });
const quoteLine = Decoration.line({ class: 'cm-md-quote' });
const inlineCode = Decoration.mark({ class: 'cm-md-inline-code' });

/** Code blocks get a monospaced, shaded band and quotes a bar, line by line; inline code a shaded box. */
function decorate(view: EditorView): DecorationSet {
  const lines = new Map<number, Decoration>();
  const marks: { from: number; to: number }[] = [];
  const { state } = view;
  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter: (node) => {
        const line = node.name === 'FencedCode' || node.name === 'CodeBlock' ? codeLine : node.name === 'Blockquote' ? quoteLine : null;
        if (line) {
          for (let pos = node.from; pos <= node.to;) {
            const l = state.doc.lineAt(pos);
            if (!lines.has(l.from)) lines.set(l.from, line);
            pos = l.to + 1;
          }
          return line === quoteLine;
        }
        if (node.name === 'InlineCode') marks.push({ from: node.from, to: node.to });
        return true;
      },
    });
  }
  // Lines and marks go in by position; a line decoration sits at its line's start, before any mark there.
  const all = [
    ...[...lines].map(([at, d]) => ({ from: at, to: at, d })),
    ...marks.map((m) => ({ ...m, d: inlineCode })),
  ].sort((a, b) => a.from - b.from || (a.from === a.to ? -1 : 0) - (b.from === b.to ? -1 : 0));
  const builder = new RangeSetBuilder<Decoration>();
  for (const r of all) builder.add(r.from, r.to, r.d);
  return builder.finish();
}

const blockDecorations = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = decorate(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) this.decorations = decorate(u.view);
    }
  },
  { decorations: (p) => p.decorations },
);
