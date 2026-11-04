import { Lock } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Chip, EmptyState } from '@ui';
import type { AreaDef } from '../api/gen/AreaDef';
import { LabelsEditor } from './LabelsEditor';
import { useProjectAreas } from './queries';

const UNKNOWN_AREA_TITLE = 'Not in .pdt/project.toml — ignored for overlap';

/**
 * One area. With the project's configuration loaded (`config`), an area it doesn't declare is struck through, and an
 * exclusive one carries a lock.
 */
function AreaChip({ name, config }: { name: string; config?: AreaDef[] }) {
  const def = config?.find((a) => a.name === name);
  const unknown = config !== undefined && !def;
  const title = unknown
    ? UNKNOWN_AREA_TITLE
    : def
      ? `${def.exclusive ? 'Exclusive: one active ticket at a time. ' : ''}${def.paths.join(', ')}`
      : undefined;
  return (
    <Chip
      title={title}
      icon={def?.exclusive ? <Lock aria-label="Exclusive" /> : undefined}
      className={cn(unknown && 'text-muted-foreground line-through')}
    >
      {name}
    </Chip>
  );
}

/** Area chips (read only). */
export function AreaChips({ areas, config, className }: { areas: string[]; config?: AreaDef[]; className?: string }) {
  if (areas.length === 0) return null;
  return (
    <span className={cn('flex min-w-0 flex-wrap items-center gap-1', className)}>
      {areas.map((a) => <AreaChip key={a} name={a} config={config} />)}
    </span>
  );
}

/**
 * A ticket's areas, editable: the project's configured areas are offered; others are kept but struck through. A
 * configuration error shows quietly underneath.
 */
export function AreasEditor({ projectId, areas, onChange, disabled }: {
  projectId: string | null;
  areas: string[];
  onChange: (areas: string[]) => void;
  disabled?: boolean;
}) {
  const q = useProjectAreas(projectId);
  const config = q.data && !q.data.error ? q.data.areas : undefined;
  return (
    <div className="flex flex-col gap-1">
      <LabelsEditor
        noun="Area"
        labels={areas}
        suggestions={config?.map((a) => a.name) ?? []}
        onChange={onChange}
        disabled={disabled}
        renderChip={(a) => <AreaChip name={a} config={config} />}
      />
      {q.data?.error ? (
        <p className="text-meta text-muted-foreground" title={q.data.error}>
          <span className="line-clamp-2">Areas config: {q.data.error}</span>
        </p>
      ) : q.data && q.data.areas.length === 0 ? (
        <EmptyState size="inline">No areas set up for this project yet.</EmptyState>
      ) : null}
    </div>
  );
}
