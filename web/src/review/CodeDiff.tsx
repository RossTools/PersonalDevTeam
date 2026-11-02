import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Compartment, EditorState, RangeSet, StateEffect, StateField, Text, type Extension } from '@codemirror/state';
import { Decoration, EditorView, GutterMarker, WidgetType, gutter, gutterLineClass, lineNumbers, type DecorationSet } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import { Chunk, MergeView, getChunks, presentableDiff, uncollapseUnchanged, unifiedMergeView } from '@codemirror/merge';
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
  /** "+" on a line (on the picked lines when it is one of them), or `c` with lines picked or selected. */
  onComment: (range: LineRange) => void;
  /**
   * Touch (phones): a tap on a line number outside the pick extends it (there is no Shift), a tap inside drops it, and
   * the picked lines show "+".
   */
  touch?: boolean;
  /** A line to show (a link to it): its collapsed run opens, it is picked and scrolled to the middle of the screen. */
  reveal?: { side: CommentSide; line: number } | null;
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

/** Lines picked in the line-number gutter (a click, then Shift+click to extend from `anchor`). */
interface Picked {
  anchor: number;
  line: number;
  endLine: number;
}

const setPicked = StateEffect.define<Picked | null>();

const pickedLine = Decoration.line({ class: 'cm-picked-line' });

/** The picked lines, tinted. */
const pickedField = StateField.define<{ picked: Picked | null; deco: DecorationSet }>({
  create: () => ({ picked: null, deco: Decoration.none }),
  update(value, tr) {
    for (const e of tr.effects) {
      if (!e.is(setPicked)) continue;
      const p = e.value;
      const doc = tr.state.doc;
      if (!p || p.line < 1 || p.endLine > doc.lines) return { picked: null, deco: Decoration.none };
      const ranges = [];
      for (let n = p.line; n <= p.endLine; n++) ranges.push(pickedLine.range(doc.line(n).from));
      return { picked: p, deco: Decoration.set(ranges) };
    }
    return value;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

class PickedGutter extends GutterMarker {
  elementClass = 'cm-picked-gutter';
}
const PICKED_GUTTER = new PickedGutter();

/** Tints the picked lines' gutter cells too. */
const pickedGutter = gutterLineClass.compute([pickedField], (state) => {
  const p = state.field(pickedField).picked;
  if (!p) return RangeSet.empty;
  const marks = [];
  for (let n = p.line; n <= p.endLine && n <= state.doc.lines; n++) marks.push(PICKED_GUTTER.range(state.doc.line(n).from));
  return RangeSet.of(marks);
});

/** Lines `anchor`..`to` in order. */
export function pickRange(anchor: number, to: number): Picked {
  return { anchor, line: Math.min(anchor, to), endLine: Math.max(anchor, to) };
}

/** Where the collapsed unchanged runs (`@codemirror/merge`'s widget) that hide any of `positions` start. */
export function collapsedHolding(view: EditorView, positions: number[]): number[] {
  const out = new Set<number>();
  if (positions.length === 0) return [];
  for (const source of view.state.facet(EditorView.decorations)) {
    if (typeof source === 'function') continue;
    source.between(0, view.state.doc.length, (from, to, deco) => {
      const type = (deco.spec.widget as { type?: string } | undefined)?.type;
      if (type === 'collapsed-unchanged-code' && positions.some((p) => p >= from && p <= to)) out.add(from);
    });
  }
  return [...out];
}

/** A position in one side of a split diff, in the other (as the merge view maps a collapsed run's start). */
function otherSidePos(pos: number, chunks: readonly Chunk[], isA: boolean): number {
  let ours = 0;
  let theirs = 0;
  for (const c of chunks) {
    if ((isA ? c.fromA : c.fromB) >= pos) break;
    [ours, theirs] = isA ? [c.toA, c.toB] : [c.toB, c.toA];
  }
  return theirs + (pos - ours);
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

/** How much of a line pair must stay the same for its changed words to be marked (else the line just changed). */
const SIMILAR = 0.5;

/**
 * GitHub-style changed words: inside each chunk, the n-th removed line is paired with the n-th added line, and when the
 * two are mostly the same, the words that differ are marked (document positions on each side). Lines without a partner
 * (wholly added or removed) and pairs that are mostly different get no marks: the line colour says enough.
 */
export function wordChanges(chunks: readonly Chunk[], a: Text, b: Text): { a: [number, number][]; b: [number, number][] } {
  const out = { a: [] as [number, number][], b: [] as [number, number][] };
  const lines = (doc: Text, from: number, to: number, end: number) => {
    if (to <= from) return [];
    const first = doc.lineAt(Math.min(from, doc.length)).number;
    const last = doc.lineAt(Math.min(end, doc.length)).number;
    return Array.from({ length: last - first + 1 }, (_, i) => doc.line(first + i));
  };
  for (const c of chunks) {
    const la = lines(a, c.fromA, c.toA, c.endA);
    const lb = lines(b, c.fromB, c.toB, c.endB);
    for (let i = 0; i < Math.min(la.length, lb.length); i++) {
      const [x, y] = [la[i]!, lb[i]!];
      const changes = presentableDiff(x.text, y.text);
      const changed = changes.reduce((n, ch) => n + Math.max(ch.toA - ch.fromA, ch.toB - ch.fromB), 0);
      if (changes.length === 0 || 1 - changed / Math.max(x.length, y.length, 1) < SIMILAR) continue;
      for (const ch of changes) {
        if (ch.toA > ch.fromA) out.a.push([x.from + ch.fromA, x.from + ch.toA]);
        if (ch.toB > ch.fromB) out.b.push([y.from + ch.fromB, y.from + ch.toB]);
      }
    }
  }
  return out;
}

const wordMarks = (ranges: [number, number][], cls: string) =>
  EditorView.decorations.of(Decoration.set(ranges.map(([f, t]) => Decoration.mark({ class: cls }).range(f, t)), true));

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
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 12px', minWidth: '40px', cursor: 'pointer' },
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
  // The colour goes on the line (and a unified view's removed block) only. The merge view also wraps the text inside a
  // changed line in <ins>/<del> marks (and each removed line in a div); colouring those too paints it twice behind
  // the text.
  '.cm-line.cm-changedLine, &.cm-merge-b .cm-line.cm-changedLine': { backgroundColor: v('diff-add') },
  '&.cm-merge-a .cm-line.cm-changedLine, .cm-deletedChunk': { backgroundColor: v('diff-del') },
  'ins.cm-insertedLine, del.cm-deletedLine, .cm-deletedChunk .cm-deletedLine': {
    backgroundColor: 'transparent', textDecoration: 'none',
  },
  '.cm-word-add': { backgroundColor: v('diff-add-strong'), borderRadius: '2px' },
  '.cm-word-del': { backgroundColor: v('diff-del-strong'), borderRadius: '2px' },
  '.cm-deletedChunk del': { textDecoration: 'none' },
  '.cm-collapsedLines': {
    background: v('diff-hunk'), color: v('diff-hunk-fg'), fontFamily: v('font-sans'), fontSize: '12px', padding: '4px 12px',
    cursor: 'pointer',
  },
  '.cm-collapsedLines:hover': { color: v('accent') },
  '.cm-line.cm-picked-line': { backgroundImage: `linear-gradient(${v('diff-picked')}, ${v('diff-picked')})` },
  '.cm-gutterElement.cm-picked-gutter': { backgroundImage: `linear-gradient(${v('diff-picked')}, ${v('diff-picked')})` },
  // The read-only editor hides the caret (codeTheme), and caret-color is inherited: the comment boxes under the lines
  // get it back.
  '.cm-review-anchor': {
    whiteSpace: 'normal', fontFamily: v('font-sans'), padding: '6px 8px', maxWidth: '48rem', cursor: 'auto',
    caretColor: v('fg'),
  },
});

/** Touch: there is no hover, so the picked lines show their "+". */
const touchTheme = EditorView.theme({
  '.cm-comment-gutter .cm-gutterElement.cm-picked-gutter .cm-comment-plus': { display: 'block' },
  '.cm-comment-gutter .cm-gutterElement.cm-picked-gutter .cm-sign': { visibility: 'hidden' },
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
 * `wholeFile` (a run that holds a thread is expanded). Threads and the composer render under their lines. A click on a
 * line number picks that line and Shift+click extends the pick; "+" in the gutter (on the picked lines when it is one
 * of them) or `c` on picked or selected lines asks for a new comment.
 */
export function CodeDiff({ path, oldText, newText, mode, wholeFile, anchors, renderAnchor, onComment, touch = false, reveal = null }: CodeDiffProps) {
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
    const at = (view: EditorView, lines: number[]) =>
      lines.filter((n) => n >= 1 && n <= view.state.doc.lines).map((n) => view.state.doc.line(n).to);
    if (vs.old) {
      const old = list.filter((a) => a.side === 'old').map(({ id, line }) => ({ id, line }));
      const neu = list.filter((a) => a.side === 'new').map(({ id, line }) => ({ id, line }));
      vs.old.dispatch({ effects: setAnchors.of(old) });
      vs.new.dispatch({ effects: setAnchors.of(neu) });
      // A thread inside a collapsed run would be hidden: expand the run on both sides.
      const chunks = getChunks(vs.new.state)?.chunks ?? [];
      const starts = [
        ...collapsedHolding(vs.old, at(vs.old, old.map((a) => a.line))).map((p) => [p, otherSidePos(p, chunks, true)] as const),
        ...collapsedHolding(vs.new, at(vs.new, neu.map((a) => a.line))).map((p) => [otherSidePos(p, chunks, false), p] as const),
      ];
      if (starts.length) {
        vs.old.dispatch({ effects: starts.map(([a]) => uncollapseUnchanged.of(a)) });
        vs.new.dispatch({ effects: starts.map(([, b]) => uncollapseUnchanged.of(b)) });
      }
    } else {
      const chunks = getChunks(vs.new.state)?.chunks ?? [];
      const a = EditorState.create({ doc: oldText }).doc;
      const mapped = list.map(({ id, side, line }) => ({ id, line: side === 'old' ? oldToNewLine(chunks, a, vs.new.state.doc, line) : line }));
      vs.new.dispatch({ effects: setAnchors.of(mapped) });
      const starts = collapsedHolding(vs.new, at(vs.new, mapped.map((m) => m.line)));
      if (starts.length) vs.new.dispatch({ effects: starts.map((p) => uncollapseUnchanged.of(p)) });
    }
  };

  /** Picks lines in `view` (and none in the other side). */
  const pick = (view: EditorView, p: Picked | null) => {
    const vs = views.current;
    for (const other of [vs?.old, vs?.new]) {
      if (other && other !== view && other.state.field(pickedField).picked) other.dispatch({ effects: setPicked.of(null) });
    }
    view.dispatch({ effects: setPicked.of(p) });
  };

  /** Asks for a comment on lines of `view`, and drops the pick. */
  const commentOn = (view: EditorView, s: CommentSide, line: number, endLine: number) => {
    if (view.state.field(pickedField).picked) view.dispatch({ effects: setPicked.of(null) });
    onCommentRef.current({ side: s, line, endLine, lineText: textOf(view.state.doc, line, endLine) });
  };

  useEffect(() => {
    const parent = host.current!;
    const collapse = wholeFile ? undefined : { margin: 3, minSize: 4 };
    // The same diff the merge view draws (its default scan limit), for the gutters' signs and tints.
    const docA = Text.of(oldText.split('\n'));
    const docB = Text.of(newText.split('\n'));
    const chunks = Chunk.build(docA, docB, { scanLimit: 500 });
    const words = wordChanges(chunks, docA, docB);
    const delLines = changedLines(chunks, docA, 'a');
    const addLines = changedLines(chunks, docB, 'b');
    const lineMarks = (doc: Text, lines: Set<number>, mark: LineClass) =>
      gutterLineClass.of(RangeSet.of([...lines].sort((x, y) => x - y).map((n) => mark.range(doc.line(n).from))));
    const side = (s: CommentSide, language: Compartment, changed: Set<number>): Extension[] => [
      lineNumbers({
        domEventHandlers: {
          mousedown(view, block, event) {
            const n = view.state.doc.lineAt(block.from).number;
            const current = view.state.field(pickedField).picked;
            const shift = (event as MouseEvent).shiftKey;
            const inside = !!current && n >= current.line && n <= current.endLine;
            if (touch && current) pick(view, inside ? null : pickRange(current.anchor, n));
            else if (shift && current) pick(view, pickRange(current.anchor, n));
            else if (!shift && current && current.line === n && current.endLine === n) pick(view, null);
            else pick(view, pickRange(n, n));
            view.contentDOM.focus({ preventScroll: true });
            return true;
          },
        },
      }),
      gutter({
        class: 'cm-comment-gutter',
        lineMarker: (view, block) => SIGNS[changed.has(view.state.doc.lineAt(block.from).number) ? (s === 'old' ? '−' : '+') : ''],
        initialSpacer: () => SIGNS['+'],
        domEventHandlers: {
          mousedown(view, block, event) {
            const n = view.state.doc.lineAt(block.from).number;
            const p = view.state.field(pickedField).picked;
            if (p && (event as MouseEvent).shiftKey) {
              const r = pickRange(p.anchor, n);
              commentOn(view, s, r.line, r.endLine);
            } else if (p && n >= p.line && n <= p.endLine) commentOn(view, s, p.line, p.endLine);
            else commentOn(view, s, n, n);
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
      touch ? touchTheme : [],
      githubHighlight,
      language.of([]),
      anchorField(container),
      pickedField,
      pickedGutter,
    ];
    const langOld = new Compartment();
    const langNew = new Compartment();
    let merge: MergeView | null = null;
    let single: EditorView | null = null;
    if (mode === 'split') {
      merge = new MergeView({
        a: { doc: oldText, extensions: [side('old', langOld, delLines), lineMarks(docA, delLines, DEL_LINE), wordMarks(words.a, 'cm-word-del')] },
        b: { doc: newText, extensions: [side('new', langNew, addLines), lineMarks(docB, addLines, ADD_LINE), wordMarks(words.b, 'cm-word-add')] },
        parent,
        highlightChanges: false,
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
            wordMarks(words.b, 'cm-word-add'),
            unifiedMergeView({ original: oldText, mergeControls: false, highlightChanges: false, gutter: false, syntaxHighlightDeletions: true, ...(collapse ? { collapseUnchanged: collapse } : {}) }),
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
  }, [path, oldText, newText, mode, wholeFile, touch]);

  const anchorKey = anchors.map((a) => `${a.id}:${a.side}:${a.line}`).join('|');
  useEffect(() => {
    for (const a of anchorsRef.current) roRef.current?.observe(container(a.id));
    place();
  }, [anchorKey]);

  // After the editors are built (the effects above run first), and again when the line asked for changes.
  const revealKey = reveal ? `${reveal.side}:${reveal.line}:${mode}:${wholeFile}` : '';
  useEffect(() => {
    const vs = views.current;
    if (!reveal || !vs) return;
    let view = vs.new;
    let line = reveal.line;
    if (reveal.side === 'old') {
      if (vs.old) view = vs.old;
      else line = oldToNewLine(getChunks(vs.new.state)?.chunks ?? [], EditorState.create({ doc: oldText }).doc, vs.new.state.doc, line);
    }
    if (line < 1 || line > view.state.doc.lines) return;
    const pos = view.state.doc.line(line).from;
    const starts = collapsedHolding(view, [pos]);
    if (starts.length) view.dispatch({ effects: starts.map((p) => uncollapseUnchanged.of(p)) });
    pick(view, pickRange(line, line));
    view.dispatch({ effects: EditorView.scrollIntoView(view.state.doc.line(line).from, { y: 'center' }) });
  }, [revealKey]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'c' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement;
    if (t.closest('textarea, input, [contenteditable="true"], .cm-review-anchor')) return;
    const vs = views.current;
    if (!vs) return;
    const sideOf = (view: EditorView): CommentSide => (view === vs.old ? 'old' : 'new');
    const picked = [vs.old, vs.new].find((x) => x?.state.field(pickedField).picked);
    if (picked) {
      const p = picked.state.field(pickedField).picked!;
      e.preventDefault();
      commentOn(picked, sideOf(picked), p.line, p.endLine);
      return;
    }
    const view = [vs.old, vs.new].find((x) => x?.hasFocus) ?? [vs.new, vs.old].find((x) => x && !x.state.selection.main.empty);
    if (!view) return;
    const { line, endLine } = selectedLines(view);
    e.preventDefault();
    commentOn(view, sideOf(view), line, endLine);
  };

  return (
    <div ref={host} onKeyDown={onKeyDown} data-testid="code-diff" className="min-h-0 [&_.cm-mergeView]:min-h-full">
      {anchors.map((a) => createPortal(renderAnchor(a.id), container(a.id), a.id))}
    </div>
  );
}
