import { Badge, EmptyState, List, ListRow, Skeleton } from '@ui';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { useAgents } from '../common/agents';
import { agentLine, sortPersonas, usePersonas } from './model';
import { LoadError } from '../common/QueryStatus';
import { ListColumnHeader } from '../common/ListColumnHeader';

/**
 * The list column of the Personas view: a heading with New persona, then every persona (name; role,
 * agent and model below), archived ones last and marked. `onSelect('new')` opens the create form (not on a phone,
 * where personas are read-only).
 */
export function PersonasList({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const personas = usePersonas();
  const agents = useAgents();
  const list = sortPersonas(personas.data ?? []);
  const phone = useMediaQuery(PHONE_QUERY);
  return (
    <div className="flex flex-col gap-0.5 pb-1">
      <ListColumnHeader title="Personas" newLabel="New persona" {...(phone ? {} : { onNew: () => onSelect('new') })} />
      {personas.isPending && <Skeleton lines={4} label="Loading personas" />}
      <LoadError what="Personas" error={personas.error} />
      {personas.isSuccess && (
        <List
          aria-label="Personas"
          onAction={onSelect}
          renderEmpty={() => <EmptyState>No personas yet.</EmptyState>}
        >
          {list.map((p) => (
            <ListRow
              key={p.id}
              id={p.id}
              title={p.name}
              textValue={p.name}
              current={selectedId === p.id}
              secondary={[p.role, agentLine(p, agents.label)].filter(Boolean).join(' · ')}
              trailing={p.archived ? <Badge>Archived</Badge> : undefined}
              className={p.archived ? 'text-muted-foreground' : undefined}
            />
          ))}
        </List>
      )}
    </div>
  );
}
