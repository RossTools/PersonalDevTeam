import { lazy, Suspense, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type TouchEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronLeft, ChevronRight, Files } from 'lucide-react';
import {
  Alert, Badge, Button, Checkbox, CommentComposer, CommentThread, DiffStat, EmptyState, IconButton, SegmentedControl, Sheet, Skeleton, SplitPane,
  relativeTime, toast,
} from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { ReviewComment } from '../api/gen/ReviewComment';
import type { Ticket } from '../api/gen/Ticket';
import { isMarkdownPath, Markdown } from '../common/Markdown';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { actionError, runAction } from '../common/useAction';
import { useBackCloses } from '../shell/router';
import { PrBadge } from '../tickets/PullRequest';
import { invalidateTickets, useTicket } from '../tickets/queries';
import { anchorLabel, linesOf, relocate, threadsOf, type Thread } from './anchors';
import type { DiffAnchor, LineRange } from './CodeDiff';
import { FileList, PhoneFileList } from './FileList';
import { MergeDialog } from './MergeDialog';
import { invalidateReview, useChanges, useFileSides, useReviewComments } from './queries';
import { ReviewBar } from './ReviewBar';
import { useViewed } from './viewed';

// CodeMirror and its merge view load the first time a diff is shown.
const CodeDiff = lazy(() => import('./CodeDiff').then((m) => ({ default: m.CodeDiff })));

type Mode = 'split' | 'unified';
const MODE_KEY = 'pdt.review.mode';
const COMPOSER = '__composer';

function loadMode(): Mode {
  try {
    return localStorage.getItem(MODE_KEY) === 'unified' ? 'unified' : 'split';
  } catch {
    return 'split';
  }
}

const isTyping = (t: EventTarget) => t instanceof HTMLElement && !!t.closest('textarea, input, [contenteditable="true"]');

/**
 * A worktree's changes against its base branch (review spec §2): the review bar, the changed files and the selected
 * file's diff with its comment threads. `ticket` is given in the ticket tab; a changes tab reads the attempt's own.
 * On a phone (PDT-116) the files are a screen of their own and a file opens its unified diff as the next screen (Back
 * returns to the list); a swipe moves to the next or previous file, and a new comment opens in a bottom sheet.
 */
export function ChangesView({ worktreeId, ticket }: { worktreeId: string; ticket?: Ticket }) {
  const qc = useQueryClient();
  const changes = useChanges(worktreeId);
  const comments = useReviewComments(worktreeId);
  const ticketQ = useTicket(ticket ? null : changes.data?.ticketId ?? null);
  const t = ticket ?? ticketQ.data;
  const phone = useMediaQuery(PHONE_QUERY);
  const narrow = useMediaQuery('(max-width: 899.98px)');
  const [modePref, setModePref] = useState<Mode>(loadMode);
  const mode: Mode = narrow ? 'unified' : modePref;
  const [wholeFile, setWholeFile] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  // Phones: a file's screen is open (else the list), and which way the last move went (for the slide).
  const [fileOpen, setFileOpen] = useState(false);
  const [dir, setDir] = useState<1 | -1>(1);
  const [composer, setComposer] = useState<LineRange | null>(null);
  const [sending, setSending] = useState(false);
  const [merging, setMerging] = useState(false);
  const { isViewed, setViewed } = useViewed(worktreeId);

  const files = changes.data?.files ?? [];
  const file = files.find((f) => f.path === selected) ?? files[0] ?? null;
  // The file shown first stays shown when another file comes before it (an agent adds one), instead of switching under
  // the user (and dropping a comment being written).
  useEffect(() => {
    if (file && selected !== file.path && !files.some((f) => f.path === selected)) setSelected(file.path);
  }, [file, selected, files]);
  useEffect(() => setComposer(null), [file?.path]);
  const threads = useMemo(() => threadsOf(comments.data ?? []), [comments.data]);
  const threadCounts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const th of threads) m[th.root.path] = (m[th.root.path] ?? 0) + 1;
    return m;
  }, [threads]);
  const drafts = (comments.data ?? []).filter((c) => c.sentAt === null).length;

  useBackCloses(phone && fileOpen && file !== null, () => setFileOpen(false));

  const select = (path: string) => {
    setSelected(path);
    setDir(1);
    setFileOpen(true);
  };
  // Phones: the next or previous file, stopping at either end.
  const move = (by: 1 | -1) => {
    const i = file ? files.findIndex((f) => f.path === file.path) : -1;
    const next = files[i + by];
    if (i < 0 || !next) return;
    setDir(by);
    setSelected(next.path);
  };
  const touch = useRef<{ x: number; y: number } | null>(null);
  const onTouchStart = (e: TouchEvent) => {
    // Text boxes and threads keep their own gestures, and so do portals (the comment sheet): React bubbles their
    // touches here too, but a swipe there must not move to another file and drop the comment being written.
    const t = e.target as Element;
    const p = e.touches[0];
    const outside = !(e.currentTarget as Element).contains(t);
    touch.current = !p || outside || t.closest('textarea, input, .cm-review-anchor') ? null : { x: p.clientX, y: p.clientY };
  };
  const onTouchEnd = (e: TouchEvent) => {
    const from = touch.current;
    touch.current = null;
    const p = e.changedTouches[0];
    if (!from || !p) return;
    const dx = p.clientX - from.x;
    const dy = p.clientY - from.y;
    if (Math.abs(dx) < 60 || Math.abs(dx) < 2 * Math.abs(dy)) return;
    move(dx < 0 ? 1 : -1);
  };
  const step = (by: number) => {
    if (!file || files.length === 0) return;
    const i = files.findIndex((f) => f.path === file.path);
    const next = files[(i + by + files.length) % files.length];
    if (next) setSelected(next.path);
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target)) return;
    if (e.key === ']') step(1);
    else if (e.key === '[') step(-1);
    else if (e.key === 'v' && file) setViewed(file, !isViewed(file));
    else return;
    e.preventDefault();
  };

  const sendFeedback = async () => {
    setSending(true);
    try {
      const r = await api.sendFeedback(worktreeId);
      toast.success(`Sent ${r.sent} comment${r.sent === 1 ? '' : 's'} to the agent.`);
      invalidateReview(qc, worktreeId);
      if (t) invalidateTickets(qc, [t.id]);
    } catch (e) {
      toast.error(actionError(e));
    } finally {
      setSending(false);
    }
  };
  const approve = () =>
    void runAction(async () => {
      const id = changes.data?.attemptId;
      if (!id) return;
      await api.approveAttempt(id);
      invalidateReview(qc, worktreeId);
      if (t) invalidateTickets(qc, [t.id]);
    }, { success: 'Approved.' });

  if (changes.isPending) return <div className="p-4"><Skeleton lines={6} label="Loading changes" /></div>;
  if (!changes.data) {
    return (
      <EmptyState action={<Button size="sm" onClick={() => void changes.refetch()}>Retry</Button>}>
        {changes.error ? actionError(changes.error) : 'Could not read the changes.'}
      </EmptyState>
    );
  }
  const c = changes.data;
  const viewedCount = files.filter(isViewed).length;
  const list = (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-meta text-muted-foreground">
        <span className="min-w-0 truncate" title={`${c.branch} against ${c.base} (from ${c.mergeBase.slice(0, 8)})`}>
          {files.length} file{files.length === 1 ? '' : 's'} vs <span className="font-mono">{c.base}</span>
        </span>
        {files.length > 0 && <span className="ml-auto shrink-0">{viewedCount} / {files.length} viewed</span>}
      </div>
      {phone ? (
        <PhoneFileList files={files} onSelect={select} comments={threadCounts} viewed={isViewed} />
      ) : (
        <FileList
          files={files}
          selected={file?.path ?? null}
          onSelect={select}
          comments={threadCounts}
          viewed={isViewed}
          className="min-h-0 flex-1 overflow-auto"
        />
      )}
    </div>
  );
  const pane = file ? (
    <FilePane
      key={file.path}
      worktreeId={worktreeId}
      file={file}
      threads={threads.filter((th) => th.root.path === file.path)}
      mode={mode}
      narrow={narrow}
      phone={phone}
      onMode={(m) => {
        setModePref(m);
        try { localStorage.setItem(MODE_KEY, m); } catch { /* storage blocked */ }
      }}
      wholeFile={wholeFile}
      onWholeFile={setWholeFile}
      viewed={isViewed(file)}
      onViewed={(on) => setViewed(file, on)}
      composer={composer}
      onComposer={setComposer}
    />
  ) : (
    <EmptyState>{c.dirty ? 'Only ignored files changed.' : `Nothing changed on ${c.branch} yet.`}</EmptyState>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <ReviewBar
        drafts={drafts}
        onSend={() => void sendFeedback()}
        sending={sending}
        attemptActive={c.attemptActive}
        approved={c.approved}
        onApprove={approve}
        onMerge={() => setMerging(true)}
        leading={
          <>
            {t?.attempt?.pr && <PrBadge pr={t.attempt.pr} attemptMerged={t.attempt.state === 'merged'} />}
            {c.dirty && <Badge tone="waiting" title="The worktree has changes that aren't committed">Uncommitted</Badge>}
          </>
        }
      />
      {phone ? (
        fileOpen && file ? (
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden" onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
            <PhoneFileNav
              index={files.findIndex((f) => f.path === file.path)}
              count={files.length}
              onList={() => setFileOpen(false)}
              onMove={move}
            />
            <div
              key={file.path}
              className={cn('flex min-h-0 flex-1 flex-col animate-in duration-200 motion-reduce:animate-none', dir === 1 ? 'slide-in-from-right' : 'slide-in-from-left')}
            >
              {pane}
            </div>
          </div>
        ) : list
      ) : (
        <SplitPane id="review-files" label="Resize the file list" defaultSize={300} start={list} end={pane} />
      )}
      {merging && t && c.attemptId && <MergeDialog ticket={t} attemptId={c.attemptId} onClose={() => setMerging(false)} />}
    </div>
  );
}

/** A phone's file screen: back to the list, and the previous and next file (a swipe does the same). */
function PhoneFileNav({ index, count, onList, onMove }: { index: number; count: number; onList: () => void; onMove: (by: 1 | -1) => void }) {
  return (
    <nav aria-label="Files" className="flex shrink-0 items-center gap-1 border-b border-border px-1 py-1">
      <Button size="sm" variant="ghost" onClick={onList}><Files aria-hidden /> All files</Button>
      <span className="ml-auto text-meta text-muted-foreground" aria-live="polite">{index + 1} of {count}</span>
      <IconButton label="Previous file" noTooltip disabled={index <= 0} onClick={() => onMove(-1)}><ChevronLeft aria-hidden /></IconButton>
      <IconButton label="Next file" noTooltip disabled={index >= count - 1} onClick={() => onMove(1)}><ChevronRight aria-hidden /></IconButton>
    </nav>
  );
}

function FilePane({ worktreeId, file, threads, mode, narrow, phone, onMode, wholeFile, onWholeFile, viewed, onViewed, composer, onComposer }: {
  worktreeId: string;
  file: ChangedFile;
  threads: Thread[];
  mode: Mode;
  narrow: boolean;
  /** The composer opens in a bottom sheet, and line numbers take taps (`CodeDiff` `touch`). */
  phone: boolean;
  onMode: (m: Mode) => void;
  wholeFile: boolean;
  onWholeFile: (on: boolean) => void;
  viewed: boolean;
  onViewed: (on: boolean) => void;
  composer: LineRange | null;
  onComposer: (r: LineRange | null) => void;
}) {
  const qc = useQueryClient();
  const sides = useFileSides(worktreeId, file);
  // A Markdown file can also be read rendered: before and after side by side (split), or as it is now (unified).
  const markdown = isMarkdownPath(file.path) && !file.binary;
  const [rendered, setRendered] = useState(false);
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
  const outdated = placed.filter((p) => p.at === null).map((p) => p.th);

  const thread = (th: Thread, note?: ReactNode) => (
    <CommentThread
      comments={[th.root, ...th.replies].map((cm: ReviewComment) => ({
        id: cm.id,
        author: 'You',
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
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface-2 px-3 py-1.5">
        <span className="min-w-0 truncate font-mono text-xs" title={file.path}>
          {file.oldPath ? `${file.oldPath} → ` : ''}{file.path}
        </span>
        {file.untracked && <Badge title="Not added to git yet">untracked</Badge>}
        {!file.binary && <DiffStat added={file.added} removed={file.removed} className="text-xs" />}
        <span className="ml-auto flex items-center gap-2">
          {markdown && (
            <SegmentedControl<'diff' | 'rendered'>
              aria-label="Show"
              size="sm"
              value={rendered ? 'rendered' : 'diff'}
              onChange={(v) => setRendered(v === 'rendered')}
              options={[{ value: 'diff', label: 'Diff' }, { value: 'rendered', label: 'Rendered' }]}
            />
          )}
          {!narrow && (
            <SegmentedControl
              aria-label="Diff layout"
              size="sm"
              value={mode}
              onChange={onMode}
              options={[{ value: 'split', label: 'Split' }, { value: 'unified', label: 'Unified' }]}
            />
          )}
          {!(markdown && rendered) && (
            <Button size="sm" variant="ghost" aria-pressed={wholeFile} onClick={() => onWholeFile(!wholeFile)}>
              {wholeFile ? 'Changes only' : 'Whole file'}
            </Button>
          )}
          <Checkbox checked={viewed} onChange={onViewed}>Viewed</Checkbox>
        </span>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {outdated.length > 0 && (
          <div className="flex flex-col gap-2 border-b border-border p-3">
            {outdated.map((th) => <div key={th.root.id}>{thread(th, 'The lines this was on have changed.')}</div>)}
          </div>
        )}
        {file.binary || s?.binary ? (
          <EmptyState>A binary file; there is no text diff.</EmptyState>
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
            />
          </Suspense>
        )}
      </div>
      {phone && (
        <Sheet side="bottom" title={composer ? composerLabel(composer) : 'Comment'} open={composer !== null} onOpenChange={(o) => { if (!o) onComposer(null); }}>
          {composer && (
            <div className="flex flex-col gap-2">
              {composer.lineText && (
                <pre className="max-h-24 overflow-auto rounded-md bg-surface-2 px-2 py-1 font-mono text-xs whitespace-pre-wrap">{composer.lineText}</pre>
              )}
              {commentComposer(composer)}
            </div>
          )}
        </Sheet>
      )}
    </div>
  );
}

/** A Markdown file's two sides rendered: Before | After in the split layout, else the file as it is now (or was). */
function RenderedSides({ oldText, newText, split }: { oldText: string | null; newText: string | null; split: boolean }) {
  const side = (label: string, text: string) => (
    <section aria-label={label} className="min-w-0 flex-1 p-4">
      <h3 className="mb-2 text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</h3>
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
