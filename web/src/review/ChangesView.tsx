import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronDown, PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { Badge, Button, EmptyState, IconButton, MenuButton, SegmentedControl, Skeleton, SplitPane, toast } from '@ui';
import { api } from '../api/client';
import type { ChangedFile } from '../api/gen/ChangedFile';
import type { CommentSide } from '../api/gen/CommentSide';
import type { Ticket } from '../api/gen/Ticket';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { actionError, runAction } from '../common/useAction';
import { readViewState, useViewKey, useViewState, writeViewState } from '../common/viewState';
import { useLocation } from '../shell/router';
import { PrBadge } from '../tickets/PullRequest';
import { invalidateTickets, useTicket } from '../tickets/queries';
import { threadsOf } from './anchors';
import type { LineRange } from './CodeDiff';
import { FileList, inListOrder } from './FileList';
import { FileSection, type Mode } from './FileSection';
import { MergeDialog } from './MergeDialog';
import { invalidateReview, useChanges, useReviewComments } from './queries';
import { ReviewBar } from './ReviewBar';
import { changesState } from './state';
import { useViewed } from './viewed';
import { plural } from '../common/text';

const MODE_KEY = 'pdt.review.mode';
const LIST_KEY = 'pdt.review.fileList';

function loadMode(): Mode {
  try {
    return localStorage.getItem(MODE_KEY) === 'unified' ? 'unified' : 'split';
  } catch {
    return 'split';
  }
}

function loadListHidden(): boolean {
  try {
    return localStorage.getItem(LIST_KEY) === 'hidden';
  } catch {
    return false;
  }
}

const isTyping = (t: EventTarget) => t instanceof HTMLElement && !!t.closest('textarea, input, [contenteditable="true"]');

/** A link into the changes: `?file=<path>` and optionally `&line=<n>` (`&side=old` for a removed line). */
interface DeepLink {
  file: string;
  line: number | null;
  side: CommentSide;
}

function deepLink(loc: string): DeepLink | null {
  const i = loc.indexOf('?');
  const q = new URLSearchParams(i < 0 ? '' : loc.slice(i));
  const file = q.get('file');
  if (!file) return null;
  const line = Number(q.get('line'));
  return { file, line: Number.isInteger(line) && line > 0 ? line : null, side: q.get('side') === 'old' ? 'old' : 'new' };
}

const linkKey = (l: DeepLink | null) => (l ? `${l.file}\n${l.line ?? ''}\n${l.side}` : '');

/** How long a jump to a file keeps it at the top while the diffs around it are drawn (and change height). */
const PIN_MS = 1500;

/** Where the page was scrolled: the file at the top and how far into it. */
interface ScrollAt {
  file: string;
  offset: number;
}

/** `offset` into a section, or its top when the file is now shorter than that (new commits since). */
const within = (el: HTMLElement, offset: number) => (offset > 0 && offset < el.offsetHeight ? offset : 0);

/**
 * A worktree's changes against its base branch, as one page like a GitHub pull request's "Files
 * changed" (PDT-303): the review bar, then every changed file in order, each under a sticky header, with comment
 * threads inline. The file list beside it is a navigator: a click scrolls to the file, and the file in view is
 * highlighted as the page scrolls. `ticket` is given in the ticket tab; a changes tab reads the attempt's own. On a
 * phone the page is the same, with a "Jump to file" menu instead of the list.
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
  const [listHidden, setListHidden] = useState(loadListHidden);
  // What the page shows comes back when it is left and opened again: the file, the scroll, collapsed and loaded
  // files, and a comment being written (with its text, kept by the composer).
  const named = (what: string) => changesState(worktreeId, what);
  const [current, setCurrent] = useViewState<string | null>(named('current'), null);
  const [composer, setComposerRange] = useViewState<(LineRange & { path: string }) | null>(named('composer'), null);
  const composerTextKey = useViewKey(named('composerText'));
  const setComposer = (r: (LineRange & { path: string }) | null) => {
    writeViewState(composerTextKey, undefined);
    setComposerRange(r);
  };
  // Collapsed by the chevron or Viewed, per file version (a file that changes opens again, as Viewed resets).
  const [collapsedBy, setCollapsedBy] = useViewState<Record<string, boolean>>(named('collapsed'), {});
  const [loaded, setLoaded] = useViewState<Record<string, boolean>>(named('loaded'), {});
  const scrollKey = useViewKey(named('scroll'));
  const [reveal, setReveal] = useState<(DeepLink & { line: number }) | null>(null);
  const [sending, setSending] = useState(false);
  const [merging, setMerging] = useState(false);
  const { isViewed, setViewed } = useViewed(worktreeId);

  // In the file list's order (by folder), so the page, the list and ]/[ agree.
  const files = useMemo(() => inListOrder(changes.data?.files ?? []), [changes.data]);
  const filesRef = useRef(files);
  filesRef.current = files;
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const sections = useRef(new Map<string, HTMLElement>()).current;
  const pin = useRef<{ path: string; offset: number; until: number } | null>(null);

  const threads = useMemo(() => threadsOf(comments.data ?? []), [comments.data]);
  const threadCounts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const th of threads) m[th.root.path] = (m[th.root.path] ?? 0) + 1;
    return m;
  }, [threads]);
  const drafts = (comments.data ?? []).filter((c) => c.sentAt === null).length;

  const shown = files.find((f) => f.path === current) ?? files[0] ?? null;
  // The file in view stays when another file comes before it (an agent adds one), instead of switching under the user.
  useEffect(() => {
    if (shown && current !== shown.path) setCurrent(shown.path);
  }, [shown, current]);
  const versionKey = (f: ChangedFile) => `${f.path}\n${f.version}`;
  const isCollapsed = (f: ChangedFile) => collapsedBy[versionKey(f)] ?? isViewed(f);
  const setCollapsed = (f: ChangedFile, on: boolean) => setCollapsedBy((c) => ({ ...c, [versionKey(f)]: on }));
  // Viewed collapses the file, and unticking it opens it again (as on GitHub).
  const markViewed = (f: ChangedFile, on: boolean) => {
    setViewed(f, on);
    setCollapsed(f, on);
  };

  /**
   * Scrolls the page so `path`'s header is at the top (or `offset` below it); `hold` keeps it there while diffs above
   * it are drawn.
   */
  const goTo = (path: string, hold = true, offset = 0) => {
    setCurrent(path);
    const s = scroller.current;
    const el = sections.get(path);
    if (!s || !el) return;
    s.scrollTop = el.offsetTop + within(el, offset);
    pin.current = hold ? { path, offset, until: Date.now() + PIN_MS } : null;
  };

  // The diffs drawn around a file the page jumped to change its place: keep it at the top for a moment.
  useEffect(() => {
    const el = content.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      const p = pin.current;
      const s = scroller.current;
      const target = p && sections.get(p.path);
      if (!p || !s || !target) return;
      if (Date.now() > p.until) pin.current = null;
      else s.scrollTop = target.offsetTop + within(target, p.offset);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [files.length > 0]);

  // Scroll-spy: the file whose section is at the top of the page.
  const frame = useRef<number | null>(null);
  const spy = () => {
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      const s = scroller.current;
      if (!s) return;
      const top = s.scrollTop + 4;
      let at: string | null = null;
      let atTop = 0;
      for (const f of filesRef.current) {
        const el = sections.get(f.path);
        if (!el) continue;
        if (el.offsetTop > top && at !== null) break;
        at = f.path;
        atTop = el.offsetTop;
      }
      if (at !== null) {
        setCurrent(at);
        writeViewState(scrollKey, { file: at, offset: Math.max(0, s.scrollTop - atTop) } satisfies ScrollAt);
      }
    });
  };
  useEffect(() => () => {
    if (frame.current !== null) cancelAnimationFrame(frame.current);
  }, []);

  // A link to a file (and line): open it and scroll there, once per link.
  const loc = useLocation();
  const link = deepLink(loc);
  // Kept with the page's state: coming back to the page at the same address restores where it was, not the link.
  const handledKey = useViewKey(named('link'));
  const handled = useRef(readViewState<string>(handledKey) ?? '');
  const followed = useRef(false);
  const markHandled = (k: string) => {
    handled.current = k;
    writeViewState(handledKey, k);
  };
  useEffect(() => {
    if (!link || handled.current === linkKey(link)) return;
    const f = files.find((x) => x.path === link.file);
    if (!f) return;
    markHandled(linkKey(link));
    followed.current = true;
    setCollapsed(f, false);
    setLoaded((l) => ({ ...l, [f.path]: true }));
    setReveal(link.line !== null ? { ...link, line: link.line } : null);
    // After the file opens.
    requestAnimationFrame(() => goTo(f.path, link.line === null));
  }, [linkKey(link), files.length]);

  // Back where the page was when it was left (a link not yet followed wins). The file stays even when the diff changed
  // since; a place in it that is gone falls back to its top.
  const restored = useRef(false);
  useEffect(() => {
    if (restored.current || files.length === 0) return;
    restored.current = true;
    const at = readViewState<ScrollAt>(scrollKey);
    if (!at || followed.current || (link && handled.current !== linkKey(link)) || !files.some((f) => f.path === at.file)) return;
    requestAnimationFrame(() => goTo(at.file, true, at.offset));
  }, [files.length]);

  /** Shows `path` and puts it in the address (`?file=`), so a reload or a copied link comes back to it. */
  const pick = (path: string) => {
    goTo(path);
    const u = new URL(window.location.href);
    u.searchParams.set('file', path);
    u.searchParams.delete('line');
    u.searchParams.delete('side');
    markHandled(linkKey({ file: path, line: null, side: 'new' }));
    window.history.replaceState(window.history.state, '', u.pathname + u.search + u.hash);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target) || !shown) return;
    const i = files.findIndex((f) => f.path === shown.path);
    const next = files[i + 1];
    const prev = files[i - 1];
    if (e.key === ']') {
      if (next) goTo(next.path);
    } else if (e.key === '[') {
      if (prev) goTo(prev.path);
    } else if (e.key === 'v') {
      const on = !isViewed(shown);
      markViewed(shown, on);
      if (on && next) goTo(next.path);
    } else return;
    e.preventDefault();
  };

  const sendFeedback = async () => {
    setSending(true);
    try {
      const r = await api.sendFeedback(worktreeId);
      toast.success(`Sent ${plural(r.sent, 'comment')} to the agent.`);
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
  const toggleList = () => {
    setListHidden(!listHidden);
    try { localStorage.setItem(LIST_KEY, listHidden ? 'shown' : 'hidden'); } catch { /* storage blocked */ }
  };

  const toolbar = (
    <div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1 text-meta text-muted-foreground">
      {!phone && files.length > 0 && (
        <IconButton label={listHidden ? 'Show the file list' : 'Hide the file list'} size="sm" aria-pressed={!listHidden} onClick={toggleList}>
          {listHidden ? <PanelLeftOpen aria-hidden /> : <PanelLeftClose aria-hidden />}
        </IconButton>
      )}
      <span className="min-w-0 truncate" title={`${c.branch} against ${c.base} (from ${c.mergeBase.slice(0, 8)})`}>
        {plural(files.length, 'file')} vs <span className="font-mono">{c.base}</span>
      </span>
      {files.length > 0 && <span className="shrink-0">· {viewedCount} / {files.length} viewed</span>}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        {phone && files.length > 0 && (
          <MenuButton label="Jump to file" entries={() => files.map((f) => ({ label: f.path, onSelect: () => pick(f.path) }))}>
            <Button size="sm" variant="ghost">Jump to file <ChevronDown aria-hidden /></Button>
          </MenuButton>
        )}
        {!narrow && files.length > 0 && (
          <SegmentedControl
            aria-label="Diff layout"
            size="sm"
            value={mode}
            onChange={(m) => {
              setModePref(m);
              try { localStorage.setItem(MODE_KEY, m); } catch { /* storage blocked */ }
            }}
            options={[{ value: 'split', label: 'Split' }, { value: 'unified', label: 'Unified' }]}
          />
        )}
      </span>
    </div>
  );

  const page = files.length === 0 ? (
    <EmptyState>{c.dirty ? 'Only ignored files changed.' : `Nothing changed on ${c.branch} yet.`}</EmptyState>
  ) : (
    <div
      ref={scroller}
      tabIndex={-1}
      onScroll={spy}
      // The user scrolling lets go of a file the page jumped to.
      onWheel={() => { pin.current = null; }}
      onTouchStart={() => { pin.current = null; }}
      onMouseDown={() => { pin.current = null; }}
      data-testid="changes-page"
      className="relative min-h-0 flex-1 overflow-auto outline-none"
    >
      <div ref={content}>
        {files.map((f) => (
          <FileSection
            key={f.path}
            worktreeId={worktreeId}
            file={f}
            threads={threads.filter((th) => th.root.path === f.path)}
            mode={mode}
            phone={phone}
            scroller={scroller}
            sectionRef={(el) => {
              if (el) sections.set(f.path, el);
              else sections.delete(f.path);
            }}
            viewed={isViewed(f)}
            onViewed={(on) => markViewed(f, on)}
            collapsed={isCollapsed(f)}
            onCollapsed={(on) => setCollapsed(f, on)}
            loaded={!!loaded[f.path]}
            onLoad={() => setLoaded((l) => ({ ...l, [f.path]: true }))}
            composer={composer?.path === f.path ? composer : null}
            onComposer={(r) => setComposer(r ? { ...r, path: f.path } : null)}
            reveal={reveal?.file === f.path ? reveal : null}
          />
        ))}
      </div>
    </div>
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
      {toolbar}
      {/* The page keeps its place in the tree when the list is hidden, so it keeps its scroll and drawn diffs. */}
      {phone ? page : (
        <SplitPane
          id="review-files"
          label="Resize the file list"
          defaultSize={300}
          hideStart={listHidden || files.length === 0}
          start={
            <FileList
              files={files}
              selected={shown?.path ?? null}
              onSelect={pick}
              comments={threadCounts}
              viewed={isViewed}
              className="min-h-0 flex-1 overflow-auto"
            />
          }
          end={page}
        />
      )}
      {merging && t && c.attemptId && <MergeDialog ticket={t} attemptId={c.attemptId} onClose={() => setMerging(false)} />}
    </div>
  );
}
