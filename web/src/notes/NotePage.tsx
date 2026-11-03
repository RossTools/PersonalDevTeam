import { Suspense, useEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Check, ChevronDown, Eye, FolderGit2, Globe, Pencil, Pin, Ticket } from 'lucide-react';
import { Alert, Button, EmptyState, IconButton, Input, MenuButton, RelativeTime, SegmentedControl, Skeleton, StatusDot, useConfirm, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { PmNote } from '../api/gen/PmNote';
import type { Project } from '../api/gen/Project';
import { EndOfText } from '../common/EndOfText';
import { MarkdownEditor, type MarkdownEditorHandle } from '../common/LazyMarkdownEditor';
import { Markdown } from '../common/Markdown';
import { useAction } from '../common/useAction';
import { useRefreshPm } from '../pm/usePm';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { NewTicketDialog } from '../shell/screens';
import { ALL, HOST_WIDE, shortDate, ticketDescription, titleFor, TITLE_MAX, wordCount, type Whose } from './model';
import { useProjects } from '../projects/queries';
import { useNoteDraft, type SaveState } from './useNoteDraft';

type Mode = 'write' | 'read';

const MODE_OPTIONS = [
  { value: 'write' as const, label: 'Write', icon: <Pencil aria-hidden className="size-3" /> },
  { value: 'read' as const, label: 'Read', icon: <Eye aria-hidden className="size-3" /> },
];

export interface NoteScreenProps {
  /** The note's id, or `new` for one not created yet. */
  id: string;
  /** The list's side and scope: a new note's kind and project. */
  whose: Whose;
  scope: string;
  /** Phones: the actions go under the top bar's ⋯. */
  phone: boolean;
  /** A new note was created by its first save (replace the path with its own). */
  onCreated: (id: string) => void;
  /** The note was deleted, or is gone: back to the list. */
  onGone: () => void;
  /** The note on show is the user's (`user`) or the PM's (`note`), so the list can show its side. */
  onKind: (whose: Whose) => void;
}

/**
 * One note as a page (PDT-143), keyed so a new note keeps its editor (and the caret) when its first save gives it an
 * id and the path changes to it.
 */
export function NoteScreen(props: NoteScreenProps) {
  const created = useRef<string | null>(null);
  const fresh = useRef(0);
  const keyFor = (id: string) => (id === 'new' ? `new-${++fresh.current}` : id);
  const [shown, setShown] = useState(() => ({ id: props.id, key: keyFor(props.id) }));
  if (props.id !== shown.id) setShown({ id: props.id, key: props.id === created.current ? shown.key : keyFor(props.id) });
  return (
    <NotePage
      key={shown.key}
      {...props}
      onCreated={(id) => {
        created.current = id;
        props.onCreated(id);
      }}
    />
  );
}

function NotePage({ id, whose, scope, phone, onCreated, onGone, onKind }: NoteScreenProps) {
  const isNew = id === 'new';
  const projects = useProjects();
  const projectList = projects.data ?? [];
  const confirm = useConfirm();
  const refresh = useRefreshPm();
  const qc = useQueryClient();
  // A new note's project and pin, until its first save creates it with them.
  const [draftProject, setDraftProject] = useState<string | null>(scope !== ALL && scope !== HOST_WIDE ? scope : null);
  const [draftPinned, setDraftPinned] = useState(false);
  const draft = useNoteDraft(isNew ? null : id, { kind: whose, project: draftProject, pinned: draftPinned, onCreated });
  const { noteId, server } = draft;
  const kind: Whose = server ? (server.kind === 'user' ? 'user' : 'note') : whose;
  const mine = kind === 'user';
  const projectId = server ? server.projectId : draftProject;
  const pinned = server ? server.pinned : draftPinned;
  const project = projectList.find((p) => p.id === projectId) ?? null;
  const [mode, setMode] = useState<Mode | null>(isNew ? 'write' : null);
  const [makingTicket, setMakingTicket] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<MarkdownEditorHandle>(null);
  // What Write focuses when it opens: the title for a new or empty note, else the text (at an offset, or its end).
  const focusNext = useRef<'title' | null>(isNew ? 'title' : null);
  const [bodyFocus, setBodyFocus] = useState<number | 'end' | undefined>(undefined);
  // Where the last press in Read started, so a drag that selects text doesn't count as a double-click.
  const pressedAt = useRef<{ x: number; y: number } | null>(null);

  const serverKind = server?.kind;
  useEffect(() => {
    if (serverKind) onKind(serverKind === 'user' ? 'user' : 'note');
  }, [serverKind, onKind]);

  // An existing note opens to read when it has text, else to write (its title first).
  useEffect(() => {
    if (mode !== null || !draft.ready) return;
    if (draft.body.trim()) setMode('read');
    else {
      focusNext.current = 'title';
      setMode('write');
    }
  }, [mode, draft.ready, draft.body]);

  useEffect(() => {
    if (mode !== 'write' || !focusNext.current) return;
    titleRef.current?.focus();
    focusNext.current = null;
  }, [mode]);

  /** Opens Write focused on the title, or on the text at `at` (its end when not given). */
  const openWrite = (on: 'title' | 'body', at: number | 'end' = 'end') => {
    focusNext.current = on === 'title' ? 'title' : null;
    setBodyFocus(on === 'body' ? at : undefined);
    setMode('write');
  };
  const switchMode = (m: Mode) => {
    if (m === 'write') openWrite(draft.title.trim() ? 'body' : 'title');
    else {
      void draft.flush();
      setMode(m);
    }
  };
  // A double-click on the rendered note opens it to write, the caret at the start of the block clicked (PDT-187); not
  // on a link or a checkbox, or at the end of a drag that selected text.
  const onReadPress = (e: PointerEvent) => { pressedAt.current = { x: e.clientX, y: e.clientY }; };
  const onReadDoubleClick = (on: 'title' | 'body') => (e: MouseEvent) => {
    const from = pressedAt.current;
    if (from && Math.hypot(e.clientX - from.x, e.clientY - from.y) > 4) return;
    const target = e.target as HTMLElement;
    if (target.closest('a, button, [role="checkbox"]')) return;
    e.preventDefault();
    window.getSelection()?.removeAllRanges();
    if (on === 'title') return openWrite('title');
    const start = Number(target.closest<HTMLElement>('[data-source-start]')?.dataset.sourceStart);
    openWrite('body', Number.isFinite(start) ? start : 'end');
  };
  const onFieldKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      switchMode('read');
    }
  };

  const patch = async (p: { project?: string; pinned?: boolean }) => {
    if (!noteId) return;
    const n = await api.patchPmNote(noteId, p);
    qc.setQueryData(['pm-notes', 'one', n.id], n);
    await refresh();
  };
  const [setProject] = useAction(async (p: string | null) => {
    if (noteId) await patch({ project: p ?? '' });
    else setDraftProject(p);
  });
  const [togglePin] = useAction(async () => {
    if (noteId) await patch({ pinned: !pinned });
    else setDraftPinned((x) => !x);
  });
  const [remove, removing] = useAction(async () => {
    const { ok } = await confirm({
      title: `Delete “${titleFor(draft.title, draft.body)}”?`,
      body: mine ? 'It is gone for good.' : 'The PM no longer sees it.',
      action: 'Delete note',
      danger: true,
    });
    if (!ok) return;
    draft.stop();
    if (noteId) {
      await api.deletePmNote(noteId);
      await refresh();
    }
    onGone();
  });
  const makeTicket = () => {
    void draft.flush();
    setMakingTicket(true);
  };

  const projectEntries = (): MenuEntry[] => [
    { label: 'Host-wide', icon: projectId === null ? <Check aria-hidden /> : <Globe aria-hidden />, onSelect: () => void setProject(null) },
    ...projectList.map((p) => ({
      label: p.name,
      icon: p.id === projectId ? <Check aria-hidden /> : <FolderGit2 aria-hidden />,
      onSelect: () => void setProject(p.id),
    })),
  ];
  const pinLabel = mine ? 'Pin: listed first' : 'Pin: in every briefing';
  const created = !!noteId;
  useTopBarActions(phone ? () => [
    { submenu: `Project: ${project?.name ?? 'Host-wide'}`, entries: projectEntries() },
    { label: pinned ? 'Unpin' : mine ? 'Pin (listed first)' : 'Pin (in every briefing)', onSelect: () => void togglePin() },
    ...(mine ? [{ label: 'Make a ticket', onSelect: makeTicket, disabled: !created || projectList.length === 0 }] : []),
    { separator: true } as const,
    { label: 'Delete…', danger: true, onSelect: () => void remove(), disabled: !created },
  ] : null);

  if (draft.loadError) return <Alert tone="error" className="m-4">The note could not be loaded: {draft.loadError.message}</Alert>;
  if (server === null) {
    return (
      <EmptyState className="mt-16 border-0" action={<Button size="sm" onClick={onGone}>Back to notes</Button>}>
        This note is gone.
      </EmptyState>
    );
  }
  if (!draft.ready || mode === null) return <Skeleton lines={8} label="Loading the note" className="p-4" />;

  const words = wordCount(draft.body);
  const status = <SaveStatus state={draft.state} savedAt={server?.updatedAt ?? null} onRetry={() => void draft.flush()} />;
  const toggle = <SegmentedControl aria-label="Mode" size="sm" options={MODE_OPTIONS} value={mode} onChange={switchMode} />;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {phone ? (
        <div className="flex h-control-touch shrink-0 items-center gap-2 border-b border-border px-3">
          {status}
          <span className="ml-auto">{toggle}</span>
        </div>
      ) : (
        <div className="flex h-13 shrink-0 items-center gap-2 border-b border-border pr-3 pl-3">
          <MenuButton label="Project" entries={projectEntries} placement="bottom start">
            <Button variant="ghost" size="sm" aria-label={`Project: ${project?.name ?? 'Host-wide'}`} className="text-muted-foreground">
              {project ? <FolderGit2 aria-hidden /> : <Globe aria-hidden />}
              <span className="max-w-48 truncate">{project?.name ?? 'Host-wide'}</span>
              <ChevronDown aria-hidden />
            </Button>
          </MenuButton>
          <span className="ml-auto">{status}</span>
          {toggle}
          <IconButton label={pinLabel} pressed={pinned} onClick={() => void togglePin()} className="aria-pressed:bg-transparent aria-pressed:text-waiting aria-pressed:hover:bg-muted">
            <Pin aria-hidden />
          </IconButton>
          {mine && (
            <Button variant="secondary" size="sm" disabled={!created || projectList.length === 0} onClick={makeTicket}>
              <Ticket aria-hidden />
              Make a ticket
            </Button>
          )}
          <MenuButton label="Note actions" entries={() => [{ label: 'Delete…', danger: true, onSelect: () => void remove(), disabled: !created || removing }]} />
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {/* In Write the article fills the scroll area, its bottom room a tap target for the end of the text (PDT-264). */}
        <article className={cn('mx-auto max-w-[680px] px-5 pt-6 md:px-8 md:pt-14', mode === 'write' ? 'flex min-h-full flex-col' : 'pb-16')}>
          {mode === 'write' ? (
            <>
              <Input
                ref={titleRef}
                aria-label="Title"
                placeholder="Untitled"
                maxLength={TITLE_MAX}
                value={draft.title}
                onChange={(e) => draft.setTitle(e.target.value)}
                onBlur={() => void draft.flush()}
                onKeyDown={(e) => {
                  onFieldKey(e);
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    bodyRef.current?.focus();
                  }
                }}
                className="h-auto rounded-none border-0 bg-transparent p-0 text-title font-semibold tracking-tight shadow-none focus-visible:ring-0 pointer-coarse:h-auto pointer-coarse:text-title"
              />
              <Meta created={server?.createdAt ?? null} words={words} />
              <MarkdownEditor
                ref={bodyRef}
                label="Note"
                placeholder="Write in Markdown…"
                value={draft.body}
                onChange={draft.setBody}
                autoFocus={bodyFocus}
                onBlur={() => void draft.flush()}
                onKeyDown={onFieldKey}
              />
              <EndOfText onTap={() => bodyRef.current?.focus('end')} />
            </>
          ) : (
            <>
              <h1 className="text-title font-semibold tracking-tight break-words" onPointerDown={onReadPress} onDoubleClick={onReadDoubleClick('title')}>
                {titleFor(draft.title, draft.body)}
              </h1>
              <Meta created={server?.createdAt ?? null} words={words} />
              <div data-testid="note-body" onPointerDown={onReadPress} onDoubleClick={onReadDoubleClick('body')}>
                {draft.body.trim() ? (
                  <Markdown variant="document" breaks>{draft.body}</Markdown>
                ) : (
                  <p className="text-muted-foreground">Nothing written yet.</p>
                )}
              </div>
            </>
          )}
        </article>
      </div>
      {makingTicket && server && (
        <TicketFromNote note={{ ...server, title: titleFor(draft.title, draft.body), body: draft.body }} projects={projectList} onClose={() => setMakingTicket(false)} />
      )}
    </div>
  );
}

/** "Created 12 Oct · 214 words" under the title. */
function Meta({ created, words }: { created: number | null; words: number }) {
  return (
    <p className="mt-3 mb-7 text-xs text-muted-foreground">
      {created !== null ? `Created ${shortDate(created)}` : 'New note'} · {words === 1 ? '1 word' : `${words} words`}
    </p>
  );
}

/** Saved (and when), Saving…, or Not saved with Retry. */
function SaveStatus({ state, savedAt, onRetry }: { state: SaveState; savedAt: number | null; onRetry: () => void }) {
  return (
    <span role="status" className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      {state === 'error' ? (
        <>
          <StatusDot status="failed" label="Not saved" />
          <span className="text-destructive">Not saved</span>
          <Button variant="ghost" size="sm" onClick={onRetry}>Retry</Button>
        </>
      ) : state === 'saving' || state === 'pending' ? (
        <>
          <StatusDot status="waiting" label="Saving" />
          Saving…
        </>
      ) : savedAt !== null ? (
        <>
          <StatusDot status="running" label="Saved" />
          <span>Saved · edited <RelativeTime time={savedAt} /></span>
        </>
      ) : null}
    </span>
  );
}

/** "Make a ticket" on one of the user's notes: the new-ticket form as a draft in the note's project, prefilled. */
function TicketFromNote({ note, projects, onClose }: { note: PmNote; projects: Project[]; onClose: () => void }) {
  if (projects.length === 0) return null;
  return (
    <Suspense fallback={null}><NewTicketDialog
      heading="Make a ticket"
      projects={projects}
      projectId={note.projectId}
      title={note.title}
      description={ticketDescription(note)}
      onClose={onClose}
      onCreated={onClose}
    /></Suspense>
  );
}
