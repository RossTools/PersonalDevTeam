import { useState } from 'react';
import { CommandList, type CommandItem } from '@ui';
import type { TicketQuery } from '../api/gen/TicketQuery';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { STAGE_LABEL } from '../common/StageStepper';
import { useDebounced } from '../common/useDebounced';
import { useTicketSearch } from './queries';

export interface TicketPickerProps {
  /** One project's tickets; every project's when null. */
  projectId: string | null;
  onPick: (t: TicketSummary) => void;
  /** Tickets not offered (the ticket itself, ones already linked). */
  exclude?: string[];
  /** More of the list query (e.g. `{ parent: 'none' }`). */
  query?: Partial<TicketQuery>;
  /** Offered only when this says so. */
  filter?: (t: TicketSummary) => boolean;
  label?: string;
  placeholder?: string;
  autoFocus?: boolean;
  className?: string;
}

/**
 * A `CommandList` over tickets: type a key, a number or words (searched by the API: titles and descriptions), Up/Down
 * and Enter to pick. Lists the most recently updated tickets before anything is typed.
 */
export function TicketPicker({ projectId, onPick, exclude = [], query: extra, filter, label = 'Tickets', placeholder = 'Search by key or title…', autoFocus = true, className }: TicketPickerProps) {
  const [query, setQuery] = useState('');
  const q = useDebounced(query.trim());
  const search = useTicketSearch({ ...(projectId ? { project: projectId } : {}), ...(q ? { q } : { sort: 'updated' }), ...extra, limit: 20 });
  const items = (search.data?.items ?? []).filter((t) => !exclude.includes(t.id) && (!filter || filter(t)));
  const commands: CommandItem[] = items.map((t) => ({
    id: t.id,
    label: `${t.key} · ${t.title}`,
    detail: STAGE_LABEL[t.stage],
    // The API already matched these (descriptions too); the list's own filter must not hide them.
    keywords: query ? [query] : [],
  }));
  return (
    <CommandList
      label={label}
      placeholder={placeholder}
      items={commands}
      query={query}
      onQueryChange={setQuery}
      autoFocus={autoFocus}
      emptyText={search.isFetching ? 'Searching…' : 'No tickets found'}
      onAction={(id) => {
        const t = items.find((x) => x.id === id);
        if (t) onPick(t);
      }}
      className={className}
    />
  );
}
