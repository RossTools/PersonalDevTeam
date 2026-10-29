// One todo opened large (PDT-227): in the right panel in place of the list (full screen on a phone), with a Back
// button. Its title, its done box, the due / repeat / link controls the row has, and its notes in the Markdown editor,
// saved as you type the way notes are.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { CalendarDays, ChevronLeft, Clock, FolderGit2, Link2, Repeat } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, Checkbox, EmptyState, Input, MenuButton, StatusDot, type MenuEntry } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { MarkdownEditor } from '../common/LazyMarkdownEditor';
import type { SaveState } from '../notes/useNoteDraft';
import { DuePicker } from './DuePicker';
import { dueState, formatDay, repeatText } from './model';
import { useTodoDraft } from './useTodoDraft';

const TITLE_MAX = 500;

export function TodoView({ todo: t, now, projects, onBack, onPatch, onDelete, onLink, onRepeat, onOpenTicket }: {
  /** The todo; undefined once it is gone. */
  todo: Todo | undefined;
  now: Date;
  projects: readonly Project[];
  /** Back to the list (what is unsaved saves first). */
  onBack: () => void;
  onPatch: (p: TodoPatch) => void;
  onDelete: () => void;
  onLink: () => void;
  onRepeat: () => void;
  onOpenTicket: (t: { id: string; projectId: string }) => void;
}) {
  const draft = useTodoDraft(t);
  const [picking, setPicking] = useState(false);
  const dueRef = useRef<HTMLDivElement>(null);
  const backRef = useRef<HTMLDivElement>(null);

  // Opening a todo moves the focus into it (to Back, so Tab reaches the title and the notes next).
  useEffect(() => {
    backRef.current?.querySelector('button')?.focus();
  }, []);

  const back = () => {
    void draft.flush();
    onBack();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    back();
  };

  if (!t) {
    return (
      <EmptyState className="mt-16 border-0" action={<Button size="sm" onClick={onBack}>Back to todos</Button>}>
        This todo is gone.
      </EmptyState>
    );
  }

  const state = t.done ? null : dueState(t, now);
  const when = t.dueOn ? `${formatDay(t.dueOn, now)}${t.dueTime ? ` ${t.dueTime}` : ''}` : null;
  const ticket = t.ticketKey && t.ticketId && t.projectId ? t.ticketKey : null;
  const project = projects.find((p) => p.id === t.projectId) ?? null;
  const projectEntries = (): MenuEntry[] => [
    { label: t.projectId ? 'No project' : '✓ No project', onSelect: () => onPatch({ project: '' }) },
    ...projects.map((p) => ({ label: p.id === t.projectId ? `✓ ${p.key || p.name}` : p.key || p.name, onSelect: () => onPatch({ project: p.id }) })),
  ];
  const actions = (): MenuEntry[] => [
    ...(t.recurrence && !t.done ? [{ label: 'Stop repeating', onSelect: () => onPatch({ recurrence: null }) }] : []),
    ...(t.ticketId ? [{ label: `Unlink ${t.ticketKey ?? 'the ticket'}`, onSelect: () => onPatch({ ticket: '' }) }] : []),
    ...(t.recurrence || t.ticketId ? [{ separator: true } as const] : []),
    { label: 'Delete', danger: true, onSelect: () => { if (!t.recurrence || t.done) draft.stop(); onDelete(); } },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col" onKeyDown={onKey}>
      <div className="flex shrink-0 items-center gap-2 pt-3 pr-2 pb-2 pl-2">
        <div ref={backRef}>
          <Button variant="ghost" size="sm" onClick={back} className="text-muted-foreground">
            <ChevronLeft aria-hidden />
            Todos
          </Button>
        </div>
        <span className="ml-auto"><SaveStatus state={draft.state} onRetry={() => void draft.flush()} /></span>
        <MenuButton label={`Actions for ${t.title}`} size="sm" entries={actions} className="text-muted-foreground" />
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <article aria-label={`Todo: ${t.title}`} className="mx-auto flex max-w-[680px] flex-col gap-3 px-4 pt-2 pb-16">
          <div className="flex items-start gap-2.5">
            <Checkbox
              aria-label={t.done ? `Not done: ${t.title}` : `Done: ${t.title}`}
              shape="round"
              danger={state === 'overdue'}
              checked={t.done}
              onChange={(done) => onPatch({ done })}
              className="mt-1.5"
            />
            <Input
              aria-label="Title"
              placeholder="Title"
              maxLength={TITLE_MAX}
              value={draft.title}
              onChange={(e) => draft.setTitle(e.target.value.replace(/\n/g, ' '))}
              onBlur={() => {
                // A todo needs a title: an emptied one comes back.
                if (!draft.title.trim()) draft.setTitle(t.title);
                void draft.flush();
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  void draft.flush();
                }
              }}
              className={cn(
                'h-auto rounded-none border-0 bg-transparent p-0 text-lg font-semibold tracking-tight shadow-none focus-visible:ring-0 pointer-coarse:h-auto pointer-coarse:text-lg',
                t.done && 'text-muted-foreground line-through decoration-muted-foreground',
              )}
            />
          </div>
          <div className="flex flex-wrap items-center gap-1.5 pl-7">
            {!t.done && (
              <div ref={dueRef}>
                <Button
                  variant="secondary"
                  size="sm"
                  aria-label={when ? `Due ${when}: change` : 'Add a due date'}
                  onClick={() => setPicking(true)}
                  className={cn('font-normal', state === 'overdue' && 'text-destructive', state === 'today' && 'text-waiting')}
                >
                  {t.dueTime ? <Clock aria-hidden /> : <CalendarDays aria-hidden />}
                  {when ?? 'Due date'}
                </Button>
              </div>
            )}
            {!t.done && (
              <Button variant="secondary" size="sm" onClick={onRepeat} className="font-normal" aria-label={t.recurrence ? `Repeats ${repeatText(t.recurrence).toLowerCase()}: change` : 'Repeat…'}>
                <Repeat aria-hidden />
                {t.recurrence ? repeatText(t.recurrence) : 'Repeat'}
              </Button>
            )}
            {ticket ? (
              <Button variant="secondary" size="sm" title={t.ticketTitle ?? undefined} onClick={() => onOpenTicket({ id: t.ticketId!, projectId: t.projectId! })} className="font-mono text-meta">
                {ticket}
              </Button>
            ) : (
              <Button variant="secondary" size="sm" onClick={onLink} className="font-normal">
                <Link2 aria-hidden />
                Link a ticket
              </Button>
            )}
            <MenuButton label="Project" entries={projectEntries} placement="bottom start">
              <Button variant="secondary" size="sm" aria-label={`Project: ${project ? project.key || project.name : 'none'}`} className="font-normal">
                <FolderGit2 aria-hidden />
                {project ? project.key || project.name : 'No project'}
              </Button>
            </MenuButton>
          </div>
          <div className="mt-2">
            <MarkdownEditor
              label="Notes"
              placeholder="Write notes in Markdown…"
              value={draft.body}
              onChange={draft.setBody}
              onBlur={() => void draft.flush()}
            />
          </div>
        </article>
      </div>
      {picking && (
        <DuePicker
          open
          onOpenChange={setPicking}
          anchor={dueRef}
          label={t.title}
          value={{ dueOn: t.dueOn, dueTime: t.dueTime, recurrence: t.recurrence }}
          now={now}
          onChange={onPatch}
          onCustomRepeat={onRepeat}
        />
      )}
    </div>
  );
}

/** Saved, Saving…, or Not saved with Retry. */
function SaveStatus({ state, onRetry }: { state: SaveState; onRetry: () => void }) {
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
      ) : (
        'Saved'
      )}
    </span>
  );
}
