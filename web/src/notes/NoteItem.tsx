import { ListRow, RelativeTime } from '@ui';
import type { PmNote } from '../api/gen/PmNote';
import { ProjectKey } from '../common/BranchChip';
import { previewOf } from './model';

/**
 * One note in a `List`: its title, two lines of its text without the Markdown, then where it lives (Host-wide or the
 * project's key) and when it last changed. The open one is the list's current row.
 */
export function NoteItem({ note, projectKey, current = false }: {
  note: PmNote;
  /** The note's project key; null for a host-wide note. */
  projectKey: string | null;
  current?: boolean;
}) {
  const preview = previewOf(note.body);
  return (
    <ListRow
      id={note.id}
      textValue={note.title}
      title={<span className="font-medium">{note.title}</span>}
      current={current}
      className="px-2.5 pt-2 pb-2.5"
      details={
        <>
          {preview && <span className="line-clamp-2 leading-snug">{preview}</span>}
          <span className="mt-0.5 flex items-center gap-1.5 text-meta">
            {projectKey ? <ProjectKey projectKey={projectKey} /> : <span>Host-wide</span>}
            <span aria-hidden>·</span>
            <RelativeTime time={note.updatedAt} />
          </span>
        </>
      }
    />
  );
}
