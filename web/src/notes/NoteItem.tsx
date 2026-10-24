import { RelativeTime } from '@ui';
import { cn } from '@ui/lib/utils';
import type { PmNote } from '../api/gen/PmNote';
import { ProjectKey } from '../common/BranchChip';
import { previewOf } from './model';

/**
 * One note in the list: its title, two lines of its text without the Markdown, then where it lives (Host-wide or the
 * project's key) and when it last changed. The selected one has the soft fill and a bar on its left edge.
 */
export function NoteItem({ note, projectKey, selected = false, onOpen }: {
  note: PmNote;
  /** The note's project key; null for a host-wide note. */
  projectKey: string | null;
  selected?: boolean;
  onOpen: (id: string) => void;
}) {
  const preview = previewOf(note.body);
  return (
    <button
      type="button"
      aria-current={selected ? 'true' : undefined}
      onClick={() => onOpen(note.id)}
      className={cn(
        'relative flex w-full flex-col gap-0.5 overflow-hidden rounded-md px-2.5 pt-2 pb-2.5 text-left outline-none',
        'hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring',
        selected && 'bg-accent before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary',
      )}
    >
      <span className="truncate text-sm font-medium">{note.title}</span>
      {preview && <span className="line-clamp-2 text-xs leading-snug text-muted-foreground">{preview}</span>}
      <span className="mt-0.5 flex items-center gap-1.5 text-meta text-muted-foreground">
        {projectKey ? <ProjectKey projectKey={projectKey} /> : <span>Host-wide</span>}
        <span aria-hidden>·</span>
        <RelativeTime time={note.updatedAt} />
      </span>
    </button>
  );
}
