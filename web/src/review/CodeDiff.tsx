import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Compartment, EditorState, RangeSet, StateEffect, StateField, Text, type Extension } from '@codemirror/state';
import { Decoration, EditorView, GutterMarker, WidgetType, gutter, gutterLineClass, lineNumbers, type DecorationSet } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import { Chunk, MergeView, getChunks, unifiedMergeView } from '@codemirror/merge';
import type { CommentSide } from '../api/gen/CommentSide';
import { codeTheme, languageFor, v } from '../files/codeTheme';

/** Something shown under a line of the diff (a thread, or the new-comment composer), after line `line` of `side`. */
export interface DiffAnchor {
  id: string;
  side: CommentSide;
  line: number;
}

/** Lines picked for a new comment. */
export interface LineRange {
  side: CommentSide;
  line: number;
  endLine: number;
  lineText: string;
}

export interface CodeDiffProps {
  path: string;
  /** '' for an added file. */
  oldText: string;
  /** '' for a deleted file. */
  newText: string;
  mode: 'split' | 'unified';
  /** Show every line (no collapsing of unchanged runs). */
  wholeFile: boolean;
  anchors: DiffAnchor[];
  renderAnchor: (id: string) => ReactNode;
  /** "+" on a line, or `c` with lines selected. */
  onComment: (range: LineRange) => void;
}

const setAnchors = StateEffect.define<{ line: number; id: string }[]>();

class AnchorWidget extends WidgetType {
  constructor(readonly id: string, readonly host: (id: string) => HTMLElement) {
    super();
  }
  eq(other: AnchorWidget) {
    return other.id === this.id;
  }
  toDOM() {
    return this.host(this.id);
  }
  // The thread's own controls (text areas, buttons) handle their events, not the editor.
  ignoreEvent() {
    return true;
  }
  destroy() {}
}

/** Block widgets under lines, set with `setAnchors`. */
function anchorField(host: (id: string) => HTMLElement) {
  return StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(deco, tr) {
      let next = deco.map(tr.changes);
      for (const e of tr.effects) {
        if (!e.is(setAnchors)) continue;
        const doc = tr.state.doc;
        const ranges = e.value
          .filter((a) => a.line >= 1 && a.line <= doc.lines)
          .map((a) => ({ pos: doc.line(a.line).to, id: a.id }))
          .sort((x, y) => x.pos - y.pos)
          .map((a) => Decoration.widget({ widget: new AnchorWidget(a.id, host), block: true, side: 1 }).range(a.pos));
        next = RangeSet.of(ranges, true);
      }
      return next;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
}

/** The sign column: `+` / `−` on changed lines (as on GitHub); hovering any line shows the comment button instead. */
class SignMarker extends GutterMarker {
  constructor(readonly sign: '+' | '−' | '') {
    super();
  }
  eq(other: SignMarker) {
    return other.sign === this.sign;
  }
  toDOM() {
    const el = document.createElement('span');
    el.className = 'cm-diff-sign';
    const sign = document.createElement('span');
    sign.className = this.sign === '+' ? 'cm-sign cm-sign-add' : this.sign === '−' ? 'cm-sign cm-sign-del' : 'cm-sign';
    sign.textContent = this.sign;
    const plus = document.createElement('span');
    plus.className = 'cm-comment-plus';
    plus.textContent = '+';
    plus.title = 'Comment on this line';
    el.append(sign, plus);
    return el;
  }
}
const SIGNS = { '+': new SignMarker('+'), '−': new SignMarker('−'), '': new SignMarker('') };

/** Tints a changed line's gutter cells (line number, sign), as GitHub tints the number column. */
class LineClass extends GutterMarker {
  constructor(readonly elementClass: string) {
    super();
  }
}
const ADD_LINE = new LineClass('cm-gh-add');
const DEL_LINE = new LineClass('cm-gh-del');

/** The 1-based lines of `doc` inside the chunks' ranges on one side. */
function changedLines(chunks: readonly Chunk[], doc: Text, side: 'a' | 'b'): Set<number> {
  const out = new Set<number>();
  for (const c of chunks) {
    const [from, to, end] = side === 'a' ? [c.fromA, c.toA, c.endA] : [c.fromB, c.toB, c.endB];
    if (to <= from) continue;
    const last = doc.lineAt(Math.min(end, doc.length)).number;
    for (let n = doc.lineAt(Math.min(from, doc.length)).number; n <= last; n++) out.add(n);
  }
  return out;
}

/** GitHub's syntax colours (Primer "prettylights"), from the code tokens. */
const githubHighlight = syntaxHighlighting(HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword, t.definitionKeyword, t.moduleKeyword], color: v('code-keyword') },
  { tag: [t.string, t.special(t.string), t.regexp, t.escape], color: v('code-string') },
  { tag: [t.number, t.bool, t.null, t.atom, t.constant(t.variableName), t.attributeName, t.propertyName], color: v('code-constant') },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: v('code-comment') },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: v('code-function') },
  { tag: [t.typeName, t.className, t.namespace, t.self], color: v('code-type') },
  { tag: [t.tagName, t.heading], color: v('code-tag') },
  { tag: t.strong, fontWeight: 'bold' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.link, textDecoration: 'underline' },
  { tag: t.invalid, color: v('failed') },
]));

/** GitHub's pull request diff: soft line colours, stronger number cells and changed words, blue collapsed rows. */
const diffTheme = EditorView.theme({
  '.cm-gutters': { backgroundColor: v('bg'), borderRight: 'none' },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 12px', minWidth: '40px' },
  '.cm-gutterElement.cm-gh-add': { backgroundColor: v('diff-add-gutter'), color: v('fg') },
  '.cm-gutterElement.cm-gh-del': { backgroundColor: v('diff-del-gutter'), color: v('fg') },
  '.cm-comment-gutter .cm-gutterElement': { cursor: 'pointer', width: '20px', textAlign: 'center', position: 'relative' },
  '.cm-sign-add': { color: v('diff-add-sign') },
  '.cm-sign-del': { color: v('diff-del-sign') },
  '.cm-comment-plus': {
    display: 'none', position: 'absolute', inset: '1px 1px', borderRadius: '4px', backgroundColor: v('accent'),
    color: v('on-accent'), fontWeight: 'bold', lineHeight: '1.3',
  },
  '.cm-comment-gutter .cm-gutterElement:hover .cm-comment-plus': { display: 'block' },
  '.cm-comment-gutter .cm-gutterElement:hover .cm-sign': { visibility: 'hidden' },
  '.cm-changedLine, .cm-insertedLine, &.cm-merge-b .cm-changedLine': { backgroundColor: v('diff-add') },
  '&.cm-merge-a .cm-changedLine, .cm-deletedChunk, .cm-deletedLine': { backgroundColor: v('diff-del') },
  '.cm-changedText, &.cm-merge-b .cm-changedText, .cm-insertedLine .cm-changedText': { background: v('diff-add-strong') },
  '&.cm-merge-a .cm-changedText, .cm-deletedChunk .cm-deletedText, .cm-deletedLine .cm-changedText, .cm-deletedChunk del': {
    background: v('diff-del-strong'), textDecoration: 'none',
  },
  '.cm-collapsedLines': {
    background: v('diff-hunk'), color: v('diff-hunk-fg'), fontFamily: v('font-sans'), fontSize: '12px', padding: '4px 12px',
    cursor: 'pointer',
  },
  '.cm-collapsedLines:hover': { color: v('accent') },
  '.cm-review-anchor': {
    whiteSpace: 'normal', fontFamily: v('font-sans'), padding: '6px 8px', maxWidth: '48rem', cursor: 'auto',
  },
});

/** In a unified view, the line of the new text an old-side line shows next to. */
export function oldToNewLine(chunks: readonly Chunk[], a: Text, b: Text, oldLine: number): number {
  if (oldLine < 1 || oldLine > a.lines) return Math.min(Math.max(oldLine, 1), b.lines);
  const posA = a.line(oldLine).from;
  let delta = 0;
  for (const c of chunks) {
    if (posA < c.fromA) break;
    if (posA < c.toA) {
      if (c.toB > c.fromB) return b.lineAt(Math.min(c.endB, b.length)).number;
      return c.fromB > 0 ? b.lineAt(c.fromB - 1).number : 1;
    }
    delta = c.toB - c.toA;
  }
  return b.lineAt(Math.max(0, Math.min(posA + delta, b.length))).number;
}

/** The lines a selection covers (a selection ending at the start of a line leaves that line out). */
function selectedLines(view: EditorView): { line: number; endLine: number } {
  const sel = view.state.selection.main;
  const doc = view.state.doc;
  const first = doc.lineAt(sel.from);
  let last = doc.lineAt(sel.to);
  if (!sel.empty && sel.to === last.from && last.number > first.number) last = doc.line(last.number - 1);
  return { line: first.number, endLine: last.number };
}

function textOf(doc: Text, line: number, endLine: number): string {
  const out: string[] = [];
  for (let n = line; n <= endLine && n <= doc.lines; n++) out.push(doc.line(n).text);
  return out.join('\n');
}

/**
 * The diff of one file (`@codemirror/merge`): split (old left, new right) or unified, unchanged runs collapsed unless
 * `wholeFile`. Threads and the composer render under their lines; "+" in the gutter or `c` on a selection asks for a
 * new comment.
 */
export function CodeDiff({ path, oldText, newText, mode, wholeFile, anchors, renderAnchor, onComment }: CodeDiffProps) {
  const host = useRef<HTMLDivElement>(null);
  const containers = useRef(new Map<string, HTMLElement>()).current;
  const views = useRef<{ old: EditorView | null; new: EditorView }>(null);
  const onCommentRef = useRef(onComment);
  onCommentRef.current = onComment;
  const anchorsRef = useRef(anchors);
  anchorsRef.current = anchors;
  const roRef = useRef<ResizeObserver | null>(null);

  const container = (id: string) => {
    let el = containers.get(id);
    if (!el) {
      el = document.createElement('div');
      el.className = 'cm-review-anchor';
      el.dataset.anchor = id;
      containers.set(id, el);
    }
    return el;
  };

  /** Puts every anchor under its line, in the view showing its side. */
  const place = () => {
    const vs = views.current;
    if (!vs) return;
    const list = anchorsRef.current;
    if (vs.old) {
      vs.old.dispatch({ effects: setAnchors.of(list.filter((a) => a.side === 'old').map(({ id, line }) => ({ id, line }))) });
      vs.new.dispatch({ effects: setAnchors.of(list.filter((a) => a.side === 'new').map(({ id, line }) => ({ id, line }))) });
    } else {
      const chunks = getChunks(vs.new.state)?.chunks ?? [];
      const a = EditorState.create({ doc: oldText }).doc;
      const mapped = list.map(({ id, side, line }) => ({ id, line: side === 'old' ? oldToNewLine(chunks, a, vs.new.state.doc, line) : line }));
      vs.new.dispatch({ effects: setAnchors.of(mapped) });
    }
  };

  useEffect(() => {
    const parent = host.current!;
    const collapse = wholeFile ? undefined : { margin: 3, minSize: 4 };
    // The same diff the merge view draws (its default scan limit), for the gutters' signs and tints.
    const docA = Text.of(oldText.split('\n'));
    const docB = Text.of(newText.split('\n'));
    const chunks = Chunk.build(docA, docB, { scanLimit: 500 });
    const delLines = changedLines(chunks, docA, 'a');
    const addLines = changedLines(chunks, docB, 'b');
    const lineMarks = (doc: Text, lines: Set<number>, mark: LineClass) =>
      gutterLineClass.of(RangeSet.of([...lines].sort((x, y) => x - y).map((n) => mark.range(doc.line(n).from))));
    const side = (s: CommentSide, language: Compartment, changed: Set<number>): Extension[] => [
      lineNumbers(),
      gutter({
        class: 'cm-comment-gutter',
        lineMarker: (view, block) => SIGNS[changed.has(view.state.doc.lineAt(block.from).number) ? (s === 'old' ? '−' : '+') : ''],
        initialSpacer: () => SIGNS['+'],
        domEventHandlers: {
          mousedown(view, block) {
            const n = view.state.doc.lineAt(block.from).number;
            onCommentRef.current({ side: s, line: n, endLine: n, lineText: view.state.doc.line(n).text });
            return true;
          },
        },
      }),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      // Long lines wrap: the split view's halves are narrow, and threads under a line then fit the pane.
      EditorView.lineWrapping,
      EditorView.contentAttributes.of({ tabindex: '0', 'aria-label': `${s === 'old' ? 'Before' : 'After'}: ${path}` }),
      codeTheme,
      diffTheme,
      githubHighlight,
      language.of([]),
      anchorField(container),
    ];
    const langOld = new Compartment();
    const langNew = new Compartment();
    let merge: MergeView | null = null;
    let single: EditorView | null = null;
    if (mode === 'split') {
      merge = new MergeView({
        a: { doc: oldText, extensions: [side('old', langOld, delLines), lineMarks(docA, delLines, DEL_LINE)] },
        b: { doc: newText, extensions: [side('new', langNew, addLines), lineMarks(docB, addLines, ADD_LINE)] },
        parent,
        highlightChanges: true,
        gutter: false,
        ...(collapse ? { collapseUnchanged: collapse } : {}),
      });
      views.current = { old: merge.a, new: merge.b };
    } else {
      single = new EditorView({
        parent,
        state: EditorState.create({
          doc: newText,
          extensions: [
            ...side('new', langNew, addLines),
            lineMarks(docB, addLines, ADD_LINE),
            unifiedMergeView({ original: oldText, mergeControls: false, highlightChanges: true, gutter: false, syntaxHighlightDeletions: true, ...(collapse ? { collapseUnchanged: collapse } : {}) }),
          ],
        }),
      });
      views.current = { old: null, new: single };
    }
    place();
    let alive = true;
    void languageFor(path).then((support) => {
      if (!alive || !support) return;
      merge?.a.dispatch({ effects: langOld.reconfigure(support) });
      (merge?.b ?? single)?.dispatch({ effects: langNew.reconfigure(support) });
    });
    // Threads grow and shrink (a reply box opens): the editors measure their widgets again.
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(() => {
      views.current?.old?.requestMeasure();
      views.current?.new.requestMeasure();
    }) : null;
    for (const el of containers.values()) ro?.observe(el);
    roRef.current = ro;
    return () => {
      alive = false;
      ro?.disconnect();
      roRef.current = null;
      merge?.destroy();
      single?.destroy();
      views.current = null;
    };
    // `place` and `container` read refs; anchor changes are placed by the effect below.
  }, [path, oldText, newText, mode, wholeFile]);

  const anchorKey = anchors.map((a) => `${a.id}:${a.side}:${a.line}`).join('|');
  useEffect(() => {
    for (const a of anchorsRef.current) roRef.current?.observe(container(a.id));
    place();
  }, [anchorKey]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'c' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement;
    if (t.closest('textarea, input, [contenteditable="true"], .cm-review-anchor')) return;
    const vs = views.current;
    if (!vs) return;
    const pick = [vs.old, vs.new].find((x) => x?.hasFocus) ?? [vs.new, vs.old].find((x) => x && !x.state.selection.main.empty);
    if (!pick) return;
    const { line, endLine } = selectedLines(pick);
    e.preventDefault();
    onCommentRef.current({ side: pick === vs.old ? 'old' : 'new', line, endLine, lineText: textOf(pick.state.doc, line, endLine) });
  };

  return (
    <div ref={host} onKeyDown={onKeyDown} data-testid="code-diff" className="min-h-0 [&_.cm-mergeView]:min-h-full">
      {anchors.map((a) => createPortal(renderAnchor(a.id), container(a.id), a.id))}
    </div>
  );
}
