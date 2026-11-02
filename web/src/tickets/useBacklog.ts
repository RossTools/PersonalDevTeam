import { useCallback, useMemo, useState } from 'react';
import {
  DEFAULT_FILTERS, loadSavedFilters, READY_TO_START, storeSavedFilters, type BacklogFilters, type SavedFilter,
} from './model';

export type TicketsMode = 'backlog' | 'board';

const VIEW_KEY = 'pdt.tickets.view';

interface Stored {
  projectId: string | null;
  mode: TicketsMode;
  filters: BacklogFilters;
}

function load(): Stored {
  try {
    const v = JSON.parse(localStorage.getItem(VIEW_KEY) ?? 'null');
    if (v && typeof v === 'object') {
      return {
        projectId: typeof v.projectId === 'string' ? v.projectId : null,
        mode: v.mode === 'board' ? 'board' : 'backlog',
        filters: { ...DEFAULT_FILTERS, ...(v.filters ?? {}), q: '' },
      };
    }
  } catch {
    // Unreadable or blocked storage: the defaults.
  }
  return { projectId: null, mode: 'backlog', filters: DEFAULT_FILTERS };
}

function save(s: Stored) {
  try {
    // The search text is not remembered: it would hide tickets the next time the view opens.
    localStorage.setItem(VIEW_KEY, JSON.stringify({ ...s, filters: { ...s.filters, q: '' } }));
  } catch {
    // Storage blocked: remembered for this page only.
  }
}

/**
 * What the Tickets view shows, shared by its list column and work area and remembered per browser: the project (null:
 * all), Backlog or Board, the filters, and the saved filters.
 */
export function useBacklog() {
  const [state, setState] = useState<Stored>(load);
  const [saved, setSaved] = useState<SavedFilter[]>(loadSavedFilters);
  const update = useCallback((f: (s: Stored) => Stored) => {
    setState((s) => {
      const next = f(s);
      save(next);
      return next;
    });
  }, []);

  const setProjectId = useCallback((projectId: string | null) => update((s) => ({ ...s, projectId })), [update]);
  const setMode = useCallback((mode: TicketsMode) => update((s) => ({ ...s, mode })), [update]);
  const setFilters = useCallback(
    (f: Partial<BacklogFilters>) => update((s) => ({ ...s, filters: { ...s.filters, ...f } })),
    [update],
  );
  const resetFilters = useCallback(() => update((s) => ({ ...s, filters: DEFAULT_FILTERS })), [update]);

  const saveFilter = useCallback((name: string) => {
    setSaved((list) => {
      const next = [...list, { id: `f${Date.now().toString(36)}`, name, projectId: state.projectId, filters: state.filters }];
      storeSavedFilters(next);
      return next;
    });
  }, [state.projectId, state.filters]);
  const deleteFilter = useCallback((id: string) => {
    if (id === READY_TO_START.id) return;
    setSaved((list) => {
      const next = list.filter((f) => f.id !== id);
      storeSavedFilters(next);
      return next;
    });
  }, []);
  const applyFilter = useCallback(
    (f: SavedFilter) =>
      update((s) => ({ ...s, projectId: f.builtin ? s.projectId : f.projectId, filters: { ...DEFAULT_FILTERS, ...f.filters } })),
    [update],
  );

  return useMemo(() => ({
    ...state,
    setProjectId, setMode, setFilters, resetFilters,
    // The built-in filter comes first; the browser's own follow.
    saved: [READY_TO_START, ...saved], saveFilter, deleteFilter, applyFilter,
  }), [state, setProjectId, setMode, setFilters, resetFilters, saved, saveFilter, deleteFilter, applyFilter]);
}

export type BacklogState = ReturnType<typeof useBacklog>;
