import { useEffect, useState } from 'react';
import { CommandList, type CommandItem } from '@ui';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { STAGE_LABEL } from '../common/StageStepper';
import { useTicketSearch } from './queries';

/** Waits until typing pauses before asking the API. */
function useDebounced<T>(value: T, ms = 200): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export interface TicketPickerProps {
  /** One project's tickets; every project's when null. */
  projectId: string | null;
  onPick: (t: TicketSummary) => void;
  /** Tickets not offered (the ticket itself, ones already linked). */
  exclude?: string[];
  label?: string;
  placeholder?: string;
  autoFocus?: boolean;
  className?: string;
}

/**
 * A `CommandList` over tickets: type a key, a number or words (searched by the API: titles and descriptions), Up/Down
 * and Enter to pick. Lists the most recently updated tickets before anything is typed.
 */
export function TicketPicker({ projectId, onPick, exclude = [], label = 'Tickets', placeholder = 'Search by key or title…', autoFocus = true, className }: TicketPickerProps) {
  const [query, setQuery] = useState('');
  const q = useDebounced(query.trim());
  const search = useTicketSearch({ ...(projectId ? { project: projectId } : {}), ...(q ? { q } : { sort: 'updated' }), limit: 20 });
  const items = (search.data?.items ?? []).filter((t) => !exclude.includes(t.id));
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
