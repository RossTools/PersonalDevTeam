// The board's view menu (PDT-377): how far back the Done column (and the closed parents' lanes) reaches, saved per
// project on this host.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, ChevronDown } from 'lucide-react';
import { Button, MenuButton, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { BoardSettings } from '../api/gen/BoardSettings';
import { runAction } from '../common/useAction';

export const DAY_MS = 24 * 60 * 60 * 1000;
/** The Done windows the board offers, in days (pdt-core `board::DONE_WINDOW_DAYS`). */
export const DONE_WINDOWS = [1, 3, 7, 14, 30];
/** A project's Done window until it picks one, and the window of the board across all projects. */
export const DEFAULT_DONE_DAYS = 14;

const boardKey = (projectId: string | null) => ['board-settings', projectId] as const;

export function doneWindowLabel(days: number): string {
  return days === 1 ? 'Last day' : `Last ${days} days`;
}

/** The project's Done window in days (the default until it has loaded, and for all projects). */
export function useDoneWindow(projectId: string | null): number {
  const q = useQuery({
    queryKey: boardKey(projectId),
    queryFn: () => api.boardSettings(projectId!),
    enabled: !!projectId,
    staleTime: Infinity,
  });
  return (projectId && q.data?.doneWindowDays) || DEFAULT_DONE_DAYS;
}

/** The board's View menu: the Done window (a project's board only). */
export function BoardViewMenu({ projectId, compact }: { projectId: string; compact?: boolean }) {
  const qc = useQueryClient();
  const days = useDoneWindow(projectId);
  const save = (d: number) => void runAction(async () => {
    const s = await api.saveBoardSettings(projectId, { doneWindowDays: d });
    qc.setQueryData<BoardSettings>(boardKey(projectId), s);
  });
  const entries = (): MenuEntry[] => [
    { heading: 'Done shows' },
    ...DONE_WINDOWS.map((d): MenuEntry => ({
      label: doneWindowLabel(d),
      icon: d === days ? <Check aria-hidden /> : <span aria-hidden className="size-4" />,
      onSelect: () => save(d),
    })),
  ];
  return (
    <MenuButton label="Board view" placement="bottom end" entries={entries}>
      <Button size="sm" variant={compact ? 'ghost' : 'secondary'} aria-label={`Board view: Done shows ${doneWindowLabel(days).toLowerCase()}`}>
        Done: {doneWindowLabel(days).toLowerCase()} <ChevronDown aria-hidden />
      </Button>
    </MenuButton>
  );
}
