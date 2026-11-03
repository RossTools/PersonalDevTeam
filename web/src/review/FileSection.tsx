import { lazy, Suspense, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight } from 'lucide-react';
import {
  Alert, Badge, Button, Checkbox, CommentComposer, CommentThread, DiffStat, EmptyState, IconButton, relativeTime, SectionLabel, SegmentedControl, Sheet, Skeleton,
} from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { CommentSide } from '../api/gen/CommentSide';
import type { ReviewComment } from '../api/gen/ReviewComment';
import { Markdown } from '../common/Markdown';
import { isMarkdownPath } from '../common/markdownPath';
import { actionError, runAction } from '../common/useAction';
import { readViewState, useViewKey, useViewState, writeViewState } from '../common/viewState';

import { anchorLabel, linesOf, relocate, type Thread } from './anchors';
import type { DiffAnchor, LineRange } from './CodeDiff';
import { estimatedHeight, hiddenReason, useNearViewport } from './lazy';
import { invalidateReview, useFileSides } from './queries';
import { changesState } from './state';

// CodeMirror and its merge view load the first time a diff is shown.
const CodeDiff = lazy(() => import('./CodeDiff').then((m) => ({ default: m.CodeDiff })));

export type Mode = 'split' | 'unified';
const COMPOSER = '__composer';

export interface FileSectionProps {
  worktreeId: string;
  file: ChangedFile;
  threads: Thread[];
  /** The page's layout (Split / Unified is one setting for every file, as on GitHub). */
  mode: Mode;
  /** The composer opens in a bottom sheet, and line numbers take taps (`CodeDiff` `touch`). */
  phone: boolean;
  /** The scrolling page: diffs are drawn as they come near its visible part. */
  scroller: RefObject<HTMLElement | null>;
  sectionRef: (el: HTMLElement | null) => void;
  viewed: boolean;
  onViewed: (on: boolean) => void;
  collapsed: boolean;
  onCollapsed: (on: boolean) => void;
  /** "Load diff" was clicked (a generated or very large file's diff starts hidden). */
  loaded: boolean;
  onLoad: () => void;
  /** The new comment being written in this file, if any. */
  composer: LineRange | null;
  onComposer: (r: LineRange | null) => void;
  /** A line a link points at. */
  reveal: { side: CommentSide; line: number } | null;
}

const HIDDEN_NOTE = {
  binary: 'A binary file; there is no text diff.',
  generated: 'A generated file; its diff is hidden.',
  large: 'A large diff; it is hidden.',
} as const;

/**
 * One file on the Changes page: a header that sticks to the top while the file is on screen (its path, counts, the
 * per-file options, Viewed), then its diff with the comment threads inline. The diff is drawn when it nears the
 * visible part of the page (and dropped again far away, keeping its height), so a big change stays fast.
 */
export function FileSection(p: FileSectionProps) {
  const { file, collapsed, loaded } = p;
  const hidden = hiddenReason(file);
  const section = useRef<HTMLElement | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const near = useNearViewport(section, p.scroller);
  // A file being commented on stays drawn, and so does one holding the focus (a reply being typed).
  const [focused, setFocused] = useState(false);
  const show = !collapsed && (hidden === null || (loaded && hidden !== 'binary'));
  const drawn = show && (near || focused || p.composer !== null);
  const markdown = isMarkdownPath(file.path) && !file.binary;
  // Kept by path, so a file that changed since still shows as it was left.
  const [rendered, setRendered] = useViewState(changesState(p.worktreeId, `rendered:${file.path}`), false);
  const [wholeFile, setWholeFile] = useViewState(changesState(p.worktreeId, `wholeFile:${file.path}`), false);

  // The height it had when last drawn, to hold its place while it isn't.
  const height = useRef<number | null>(null);
  useEffect(() => {
    const el = body.current;
    if (!drawn || !el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => { height.current = el.offsetHeight; });
    ro.observe(el);
    return () => ro.disconnect();
  }, [drawn]);

  return (
    <section
      ref={(el) => {
        section.current = el;
        p.sectionRef(el);
      }}
      aria-label={file.path}
      data-path={file.path}
      className="border-b border-border"
      onFocus={() => setFocused(true)}
      onBlur={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false); }}
    >
      <header className="sticky top-0 z-(--pdt-z-sticky) flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-border bg-surface-2 px-2 py-1">
        <IconButton
          label={collapsed ? `Expand ${file.path}` : `Collapse ${file.path}`}
          noTooltip
          size="sm"
          aria-expanded={!collapsed}
          onClick={() => p.onCollapsed(!collapsed)}
        >
          {collapsed ? <ChevronRight aria-hidden /> : <ChevronDown aria-hidden />}
        </IconButton>
        <span className={cn('min-w-0 truncate font-mono text-xs', file.status === 'deleted' && 'line-through')} title={file.path} data-testid="file-name">
          {file.oldPath ? `${file.oldPath} → ` : ''}{file.path}
        </span>
        {file.untracked ? <Badge title="Not added to git yet">untracked</Badge>
          : file.uncommitted && <Badge tone="waiting" title="Changed in the worktree, not committed yet">Uncommitted</Badge>}
        {file.generated && <Badge title="Marked linguist-generated, or a lockfile">generated</Badge>}
        {!file.binary && <DiffStat added={file.added} removed={file.removed} className="text-xs" />}
        <span className="ml-auto flex items-center gap-2">
          {show && markdown && (
            <SegmentedControl<'diff' | 'rendered'>
              aria-label="Show"
              size="sm"
              value={rendered ? 'rendered' : 'diff'}
              onChange={(v) => setRendered(v === 'rendered')}
              options={[{ value: 'diff', label: 'Diff' }, { value: 'rendered', label: 'Rendered' }]}
            />
          )}
          {show && !(markdown && rendered) && (
            <Button size="sm" variant="ghost" aria-pressed={wholeFile} onClick={() => setWholeFile(!wholeFile)}>
              {wholeFile ? 'Changes only' : 'Whole file'}
            </Button>
          )}
          <Checkbox checked={p.viewed} onChange={p.onViewed}>Viewed</Checkbox>
        </span>
      </header>
      {!collapsed && hidden !== null && !show && (
        <div className="flex items-center gap-3 px-4 py-3 text-xs text-muted-foreground">
          {HIDDEN_NOTE[hidden]}
          {hidden !== 'binary' && <Button size="sm" onClick={p.onLoad}>Load diff</Button>}
        </div>
      )}
      {show && (
        <div ref={body} className="isolate" data-testid="file-body">
          {drawn ? (
            <FileDiff {...p} markdown={markdown} rendered={rendered} wholeFile={wholeFile} />
          ) : (
            <div aria-hidden style={{ height: height.current ?? estimatedHeight(file) }} />
          )}
        </div>
      )}
    </section>
  );
}

/** A file's diff (or Markdown rendered) with its threads, outdated ones above; on a phone, the composer's sheet. */
function FileDiff({ worktreeId, file, threads, mode, phone, composer, onComposer, reveal, markdown, rendered, wholeFile }: FileSectionProps & {
  markdown: boolean;
  rendered: boolean;
  wholeFile: boolean;
}) {
  const qc = useQueryClient();
  const sides = useFileSides(worktreeId, file);
  // The comment being written is kept as it is typed (ChangesView clears it when the composer closes or moves).
  const draftKey = useViewKey(changesState(worktreeId, 'composerText'));
  const refresh = () => invalidateReview(qc, worktreeId);
  const act = (f: () => Promise<unknown>) => runAction(async () => { await f(); refresh(); });
  // For a typed comment, reply or edit: a failure rejects, so the composer keeps the text and shows why.
  const save = async (f: () => Promise<unknown>) => { await f(); refresh(); };

  // Where each thread is now; the ones whose lines are gone are outdated.
  const placed = useMemo(() => {
    const s = sides.data;
    const old = linesOf(s?.old ?? '');
    const now = linesOf(s?.new ?? '');
    return threads.map((th) => {
      const at = s ? relocate(th.root, th.root.side === 'old' ? old : now) : th.root.line;
      return { th, at };
    });
  }, [threads, sides.data]);
  const anchors: DiffAnchor[] = [
    ...placed.flatMap(({ th, at }) => (at === null ? [] : [{ id: th.root.id, side: th.root.side, line: at + th.root.endLine - th.root.line }])),
    ...(composer && !phone ? [{ id: COMPOSER, side: composer.side, line: composer.endLine }] : []),
  ];
  const outdated = placed.filter((x) => x.at === null).map((x) => x.th);

  const thread = (th: Thread, note?: ReactNode) => (
    <CommentThread
      comments={[th.root, ...th.replies].map((cm: ReviewComment) => ({
        id: cm.id,
        author: cm.author === 'user' ? 'You' : cm.author,
        time: relativeTime(cm.createdAt),
        body: cm.body,
        draft: cm.sentAt === null,
      }))}
      title={note ? anchorLabel(th.root) : undefined}
      note={note}
      resolved={th.root.resolvedAt !== null}
      renderBody={(text) => <Markdown>{text}</Markdown>}
      onResolve={(resolved) => void act(() => api.updateReviewComment(th.root.id, { resolved }))}
      onReply={(text) =>
        save(() => api.createReviewComment(worktreeId, { path: th.root.path, side: th.root.side, line: th.root.line, lineText: '', body: text, parentId: th.root.id }))}
      onEdit={(id, text) => save(() => api.updateReviewComment(id, { body: text }))}
      onDelete={(id) => void act(() => api.deleteReviewComment(id))}
    />
  );
  const composerLabel = (r: LineRange) => `Comment on ${r.endLine > r.line ? `lines ${r.line}–${r.endLine}` : `line ${r.line}`}`;
  const commentComposer = (r: LineRange) => (
    <CommentComposer
      label={composerLabel(r)}
      action="Comment"
      initial={readViewState<string>(draftKey) ?? ''}
      onChange={(text) => writeViewState(draftKey, text)}
      autoFocus
      placeholder={phone ? 'Leave a comment (Markdown).' : 'Leave a comment (Markdown). Ctrl+Enter to save.'}
      onSubmit={async (text) => {
        await api.createReviewComment(worktreeId, { path: file.path, side: r.side, line: r.line, endLine: r.endLine, lineText: r.lineText, body: text });
        onComposer(null);
        refresh();
      }}
      onCancel={() => onComposer(null)}
    />
  );
  const renderAnchor = (id: string) => {
    if (id === COMPOSER && composer) return commentComposer(composer);
    const th = threads.find((x) => x.root.id === id);
    return th ? thread(th) : null;
  };

  const s = sides.data;
  return (
    <>
      {outdated.length > 0 && (
        <div className="flex flex-col gap-2 border-b border-border p-3">
          {outdated.map((th) => <div key={th.root.id}>{thread(th, 'The lines this was on have changed.')}</div>)}
        </div>
      )}
      {s?.binary ? (
        <EmptyState>{HIDDEN_NOTE.binary}</EmptyState>
      ) : sides.isPending ? (
        <Skeleton lines={8} className="p-3" label="Loading the file" />
      ) : !s ? (
        <Alert tone="error" className="m-3">{sides.error ? actionError(sides.error) : 'Could not read the file.'}</Alert>
      ) : s.tooLarge ? (
        <EmptyState>This file is over 1 MB; it isn't shown.</EmptyState>
      ) : markdown && rendered ? (
        <RenderedSides oldText={s.old} newText={s.new} split={mode === 'split'} />
      ) : (
        <Suspense fallback={<Skeleton lines={8} className="p-3" />}>
          <CodeDiff
            path={file.path}
            oldText={s.old ?? ''}
            newText={s.new ?? ''}
            mode={mode}
            wholeFile={wholeFile}
            anchors={anchors}
            renderAnchor={renderAnchor}
            onComment={(r) => onComposer(r)}
            touch={phone}
            reveal={reveal}
          />
        </Suspense>
      )}
      {phone && composer && (
        <Sheet side="bottom" title={composerLabel(composer)} open onOpenChange={(o) => { if (!o) onComposer(null); }}>
          <div className="flex flex-col gap-2">
            {composer.lineText && (
              <pre className="max-h-24 overflow-auto rounded-md bg-surface-2 px-2 py-1 font-mono text-xs whitespace-pre-wrap">{composer.lineText}</pre>
            )}
            {commentComposer(composer)}
          </div>
        </Sheet>
      )}
    </>
  );
}

/** A Markdown file's two sides rendered: Before | After in the split layout, else the file as it is now (or was). */
function RenderedSides({ oldText, newText, split }: { oldText: string | null; newText: string | null; split: boolean }) {
  const side = (label: string, text: string) => (
    <section aria-label={label} className="min-w-0 flex-1 p-4">
      <SectionLabel className="mb-2">{label}</SectionLabel>
      <Markdown>{text}</Markdown>
    </section>
  );
  if (split && oldText !== null && newText !== null) {
    return (
      <div className="flex divide-x divide-border">
        {side('Before', oldText)}
        {side('After', newText)}
      </div>
    );
  }
  return newText !== null ? side('After', newText) : side('Before', oldText ?? '');
}
