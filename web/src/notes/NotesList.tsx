import { useEffect, useRef } from 'react';
import { Brain, Pin, Search, SquarePen } from 'lucide-react';
import { Alert, EmptyState, IconButton, Input, Kbd, SegmentedControl, Select, Skeleton } from '@ui';
import { ALL, HOST_WIDE, inScope, sectionsOf, type Whose } from './model';
import { NoteItem } from './NoteItem';
import { useProjects } from '../projects/queries';
import { useNotesList, type NotesFilters } from './useNotes';

const WHOSE_OPTIONS = [
  { value: 'user' as const, label: 'Mine' },
  { value: 'note' as const, label: 'PM’s memory', icon: <Brain aria-hidden className="size-3" /> },
];

const sectionHeading = 'flex items-center gap-1.5 px-3 pt-3.5 pb-1 text-2xs font-semibold tracking-wider text-muted-foreground uppercase';

/** Typing in a field, where `/` is text rather than the search key. */
const typing = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName) || !!t.closest('[role=textbox], .xterm'));

/**
 * The Notes view's list column (PDT-143): New note, a search (`/` focuses it), Mine | PM's memory and a scope (all,
 * host-wide or a project's), then the notes in sections: Pinned, Last 7 days, Earlier. On a phone it is the view's
 * screen.
 */
export function NotesList({ filters, selectedId, onOpen, onNew, onWhose }: {
  filters: NotesFilters;
  /** The note on show. */
  selectedId: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  /** Switches between Mine and PM's memory (a path of its own). */
  onWhose: (w: Whose) => void;
}) {
  const { whose, scope, setScope, q, setQ } = filters;
  const notes = useNotesList(whose, q);
  const projects = useProjects();
  const keyOf = (id: string | null) => (id ? projects.data?.find((p) => p.id === id)?.key ?? '?' : null);
  const shown = inScope(notes.data ?? [], scope);
  const sections = sectionsOf(shown);
  const searchRef = useRef<HTMLInputElement>(null);
  const mine = whose === 'user';

  // `/` anywhere but a field focuses the search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented || typing(e.target)) return;
      e.preventDefault();
      searchRef.current?.focus();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const filtered = !!q.trim() || scope !== ALL;
  return (
    <div className="flex flex-col pb-2">
      <div className="flex items-center gap-2 pt-2.5 pr-1.5 pb-2 pl-3">
        <h2 className="flex-1 text-sm font-semibold tracking-tight">Notes</h2>
        <IconButton label="New note" onClick={onNew}>
          <SquarePen aria-hidden />
        </IconButton>
      </div>
      <div className="relative mx-2">
        <Search aria-hidden className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={searchRef}
          type="search"
          aria-label="Search notes"
          placeholder="Search notes"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape' && q) {
              e.stopPropagation();
              setQ('');
            }
          }}
          className="pr-8 pl-8"
        />
        {!q && <Kbd className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 pointer-coarse:hidden">/</Kbd>}
      </div>
      <div className="mx-2 mt-2 flex items-center gap-1.5">
        <SegmentedControl aria-label="Whose notes" size="sm" options={WHOSE_OPTIONS} value={whose} onChange={onWhose} />
        <Select
          aria-label="Scope"
          size="sm"
          quiet
          className="ml-auto w-auto max-w-32"
          options={[
            { value: ALL, label: 'All' },
            { value: HOST_WIDE, label: 'Host-wide' },
            ...(projects.data ?? []).map((p) => ({ value: p.id, label: p.name })),
          ]}
          value={scope}
          onChange={setScope}
        />
      </div>
      {!mine && (
        <p className="mx-3 mt-2 text-meta text-muted-foreground">
          What the PM knows that PDT's data can't tell it. Pinned notes go in every PM briefing.
        </p>
      )}
      {notes.isPending && <Skeleton lines={4} label="Loading notes" className="m-3" />}
      {notes.isError && <Alert tone="error" className="m-2">Notes could not be loaded: {notes.error.message}</Alert>}
      {notes.isSuccess && shown.length === 0 && (
        <EmptyState className="mt-6 border-0">
          {filtered
            ? 'No note matches.'
            : mine
              ? 'No notes yet. Start one with New note.'
              : 'The PM’s memory is empty. It saves what it learns about how you work; you can add to it too.'}
        </EmptyState>
      )}
      {sections.map((s) => (
        <section key={s.id} aria-label={s.label}>
          <h3 className={sectionHeading}>
            {s.id === 'pinned' && <Pin aria-hidden className="size-3" />}
            {s.label}
          </h3>
          <ul className="flex flex-col gap-px px-1">
            {s.notes.map((n) => (
              <li key={n.id}>
                <NoteItem note={n} projectKey={keyOf(n.projectId)} selected={n.id === selectedId} onOpen={onOpen} />
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
