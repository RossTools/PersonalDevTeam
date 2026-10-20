import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Compartment, EditorState, RangeSet, StateEffect, StateField, type Extension, type Text } from '@codemirror/state';
import { Decoration, EditorView, GutterMarker, WidgetType, gutter, lineNumbers, type DecorationSet } from '@codemirror/view';
import { MergeView, getChunks, unifiedMergeView, type Chunk } from '@codemirror/merge';
import type { CommentSide } from '../api/gen/CommentSide';
import { codeHighlight, codeTheme, languageFor, v } from '../files/codeTheme';

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

class PlusMarker extends GutterMarker {
  toDOM() {
    const el = document.createElement('span');
    el.textContent = '+';
    el.className = 'cm-comment-plus';
    el.title = 'Comment on this line';
    return el;
  }
}
const plus = new PlusMarker();

const diffTheme = EditorView.theme({
  '.cm-comment-gutter .cm-gutterElement': { cursor: 'pointer', padding: '0 4px', color: 'transparent' },
  '.cm-comment-gutter .cm-gutterElement:hover': { color: v('accent'), fontWeight: 'bold' },
  '.cm-changedLine, .cm-insertedLine, &.cm-merge-b .cm-changedLine': { backgroundColor: v('diff-add') },
  '&.cm-merge-a .cm-changedLine, .cm-deletedChunk, .cm-deletedLine': { backgroundColor: v('diff-del') },
  '.cm-changedText, &.cm-merge-b .cm-changedText, .cm-insertedLine .cm-changedText': { background: v('diff-add-strong') },
  '&.cm-merge-a .cm-changedText, .cm-deletedChunk .cm-deletedText, .cm-deletedLine .cm-changedText': { background: v('diff-del-strong') },
  '.cm-changeGutter': { width: '3px', paddingLeft: '0' },
  '.cm-changedLineGutter, &.cm-merge-b .cm-changedLineGutter': { background: v('running') },
  '&.cm-merge-a .cm-changedLineGutter, .cm-deletedLineGutter': { background: v('failed') },
  '.cm-collapsedLines': {
    background: v('surface-2'), color: v('muted'), fontFamily: v('font-sans'), fontSize: '11px', padding: '2px 8px', cursor: 'pointer',
  },
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
    const side = (s: CommentSide, language: Compartment): Extension[] => [
      lineNumbers(),
      gutter({
        class: 'cm-comment-gutter',
        lineMarker: () => plus,
        initialSpacer: () => plus,
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
      codeHighlight,
      language.of([]),
      anchorField(container),
    ];
    const langOld = new Compartment();
    const langNew = new Compartment();
    let merge: MergeView | null = null;
    let single: EditorView | null = null;
    if (mode === 'split') {
      merge = new MergeView({
        a: { doc: oldText, extensions: side('old', langOld) },
        b: { doc: newText, extensions: side('new', langNew) },
        parent,
        highlightChanges: true,
        gutter: true,
        ...(collapse ? { collapseUnchanged: collapse } : {}),
      });
      views.current = { old: merge.a, new: merge.b };
    } else {
      single = new EditorView({
        parent,
        state: EditorState.create({
          doc: newText,
          extensions: [
            ...side('new', langNew),
            unifiedMergeView({ original: oldText, mergeControls: false, highlightChanges: true, gutter: true, syntaxHighlightDeletions: true, ...(collapse ? { collapseUnchanged: collapse } : {}) }),
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
