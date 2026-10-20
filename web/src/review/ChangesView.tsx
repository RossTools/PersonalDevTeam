import { lazy, Suspense, useEffect, useMemo, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Files } from 'lucide-react';
import {
  Alert, Badge, Button, Checkbox, CommentComposer, CommentThread, DiffStat, EmptyState, SegmentedControl, Skeleton, SplitPane,
  relativeTime, toast,
} from '@ui';
import { api } from '../api/client';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { ReviewComment } from '../api/gen/ReviewComment';
import type { Ticket } from '../api/gen/Ticket';
import { Markdown } from '../common/Markdown';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { actionError, runAction } from '../common/useAction';
import { PrBadge } from '../tickets/PullRequest';
import { invalidateTickets, useTicket } from '../tickets/queries';
import { anchorLabel, linesOf, relocate, threadsOf, type Thread } from './anchors';
import type { DiffAnchor, LineRange } from './CodeDiff';
import { FileList } from './FileList';
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
  const [filesOpen, setFilesOpen] = useState(false);
  const [composer, setComposer] = useState<LineRange | null>(null);
  const [sending, setSending] = useState(false);
  const [merging, setMerging] = useState(false);
  const { isViewed, setViewed } = useViewed(worktreeId);

  const files = changes.data?.files ?? [];
  const file = files.find((f) => f.path === selected) ?? files[0] ?? null;
  useEffect(() => setComposer(null), [file?.path]);
  const threads = useMemo(() => threadsOf(comments.data ?? []), [comments.data]);
  const threadCounts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const th of threads) m[th.root.path] = (m[th.root.path] ?? 0) + 1;
    return m;
  }, [threads]);
  const drafts = (comments.data ?? []).filter((c) => c.sentAt === null).length;

  const select = (path: string) => {
    setSelected(path);
    setFilesOpen(false);
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
      <FileList
        files={files}
        selected={file?.path ?? null}
        onSelect={select}
        comments={threadCounts}
        viewed={isViewed}
        className="min-h-0 flex-1 overflow-auto"
      />
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
            {phone && (
              <Button size="sm" variant="ghost" aria-expanded={filesOpen} onClick={() => setFilesOpen((o) => !o)}>
                <Files aria-hidden /> Files ({files.length})
              </Button>
            )}
            {t?.attempt?.pr && <PrBadge pr={t.attempt.pr} attemptMerged={t.attempt.state === 'merged'} />}
            {c.dirty && <Badge tone="waiting" title="The worktree has changes that aren't committed">Uncommitted</Badge>}
          </>
        }
      />
      {phone ? (
        filesOpen ? list : pane
      ) : (
        <SplitPane id="review-files" label="Resize the file list" defaultSize={300} start={list} end={pane} />
      )}
      {merging && t && c.attemptId && <MergeDialog ticket={t} attemptId={c.attemptId} onClose={() => setMerging(false)} />}
    </div>
  );
}

function FilePane({ worktreeId, file, threads, mode, narrow, onMode, wholeFile, onWholeFile, viewed, onViewed, composer, onComposer }: {
  worktreeId: string;
  file: ChangedFile;
  threads: Thread[];
  mode: Mode;
  narrow: boolean;
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
  const refresh = () => invalidateReview(qc, worktreeId);
  const act = (f: () => Promise<unknown>) => runAction(async () => { await f(); refresh(); });

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
    ...(composer ? [{ id: COMPOSER, side: composer.side, line: composer.endLine }] : []),
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
        act(() => api.createReviewComment(worktreeId, { path: th.root.path, side: th.root.side, line: th.root.line, lineText: '', body: text, parentId: th.root.id }))}
      onEdit={(id, text) => act(() => api.updateReviewComment(id, { body: text }))}
      onDelete={(id) => void act(() => api.deleteReviewComment(id))}
    />
  );
  const renderAnchor = (id: string) => {
    if (id === COMPOSER && composer) {
      return (
        <CommentComposer
          label={`Comment on ${composer.endLine > composer.line ? `lines ${composer.line}–${composer.endLine}` : `line ${composer.line}`}`}
          action="Comment"
          autoFocus
          placeholder="Leave a comment (Markdown). Ctrl+Enter to save."
          onSubmit={async (text) => {
            const r = await runAction(() =>
              api.createReviewComment(worktreeId, { path: file.path, side: composer.side, line: composer.line, endLine: composer.endLine, lineText: composer.lineText, body: text }));
            if (r) {
              onComposer(null);
              refresh();
            }
          }}
          onCancel={() => onComposer(null)}
        />
      );
    }
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
          {!narrow && (
            <SegmentedControl
              aria-label="Diff layout"
              size="sm"
              value={mode}
              onChange={onMode}
              options={[{ value: 'split', label: 'Split' }, { value: 'unified', label: 'Unified' }]}
            />
          )}
          <Button size="sm" variant="ghost" aria-pressed={wholeFile} onClick={() => onWholeFile(!wholeFile)}>
            {wholeFile ? 'Changes only' : 'Whole file'}
          </Button>
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
            />
          </Suspense>
        )}
      </div>
    </div>
  );
}
