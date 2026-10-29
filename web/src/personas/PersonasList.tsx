import { Plus } from 'lucide-react';
import { Alert, Badge, EmptyState, IconButton, List, ListRow, Skeleton } from '@ui';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { useAgents } from '../common/agents';
import { agentLine, sortPersonas, usePersonas } from './model';

/**
 * The list column of the Personas view (UX spec §8.4): a heading with New persona, then every persona (name; role,
 * agent and model below), archived ones last and marked. `onSelect('new')` opens the create form (not on a phone,
 * where personas are read-only).
 */
export function PersonasList({ selectedId, onSelect }: { selectedId: string | null; onSelect: (id: string) => void }) {
  const personas = usePersonas();
  const agents = useAgents();
  const list = sortPersonas(personas.data ?? []);
  const phone = useMediaQuery(PHONE_QUERY);
  return (
    <div className="flex flex-col gap-0.5 py-1">
      <h2 className="flex items-center px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase">
        <span className="flex-1">Personas</span>
        {!phone && (
          <IconButton label="New persona" size="sm" noTooltip className="size-5" onClick={() => onSelect('new')}>
            <Plus aria-hidden />
          </IconButton>
        )}
      </h2>
      {personas.isPending && <Skeleton lines={4} label="Loading personas" />}
      {personas.isError && <Alert tone="error">Personas could not be loaded: {personas.error.message}</Alert>}
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
