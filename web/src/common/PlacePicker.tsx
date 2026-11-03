import type { ReactNode } from 'react';
import { useQueries, useQuery } from '@tanstack/react-query';
import { Check, ChevronDown, FolderGit2, GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { CommandList, type CommandItem } from '@ui/CommandList';
import { Popover } from '@ui/Popover';
import type { Project } from '../api/gen/Project';
import { worktreesQuery } from './worktrees';

export type Place = { projectId: string; worktreeId: string | null };

const REPOSITORY = 'Repository';
const idOf = (p: Place) => `${p.projectId}:${p.worktreeId ?? ''}`;
function placeOf(id: string): Place {
  const i = id.indexOf(':');
  const worktreeId = id.slice(i + 1);
  return { projectId: id.slice(0, i), worktreeId: worktreeId || null };
}

/** The list inside the popover: every project, each with its main checkout and its worktrees (missing ones left out). */
function Places({ projects, place, onPick }: { projects: Project[]; place: Place; onPick: (p: Place) => void }) {
  const worktrees = useQueries({
    queries: projects.map((p) => worktreesQuery(p.id)),
  });
  const current = idOf(place);
  const icon = (id: string, fallback: ReactNode) => (id === current ? <Check aria-hidden /> : fallback);
  const items: CommandItem[] = projects.flatMap((p, i) => {
    const repoId = idOf({ projectId: p.id, worktreeId: null });
    return [
      { id: repoId, label: REPOSITORY, group: p.name, keywords: [p.name, p.key, p.baseBranch], icon: icon(repoId, <FolderGit2 aria-hidden />) },
      ...(worktrees[i]?.data ?? [])
        .filter((w) => !w.missing)
        .map((w) => {
          const id = idOf({ projectId: p.id, worktreeId: w.id });
          return { id, label: w.branch, group: p.name, keywords: [p.name, p.key], icon: icon(id, <GitBranch aria-hidden />) };
        }),
    ];
  });
  return <CommandList label="Places" placeholder="Filter projects and branches…" items={items} onAction={(id) => onPick(placeOf(id))} />;
}

/**
 * Picks a project and a place in it (the main checkout or one of its worktrees): one chip "Project › place" that opens
 * a filterable list. The trigger is named "Place: <project> › <place>".
 */
export function PlacePicker({ projects, place, onPick, className }: {
  projects: Project[];
  place: Place;
  onPick: (p: Place) => void;
  className?: string;
}) {
  const worktrees = useQuery({ ...worktreesQuery(place.projectId), enabled: !!place.projectId });
  const project = projects.find((p) => p.id === place.projectId);
  const where = place.worktreeId ? (worktrees.data?.find((w) => w.id === place.worktreeId)?.branch ?? '…') : REPOSITORY;
  const projectName = project?.name ?? 'No project';
  return (
    <Popover
      label="Choose a place"
      className="w-80 p-1"
      trigger={
        <button
          type="button"
          aria-label={`Place: ${projectName} › ${where}`}
          className={cn(
            'inline-flex h-control-sm max-w-full min-w-0 items-center gap-1 rounded-md border border-input bg-background px-2 text-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:h-control-touch',
            className,
          )}
        >
          <span className="min-w-0 truncate">{projectName}</span>
          <span aria-hidden className="text-muted-foreground">›</span>
          <span className="min-w-0 truncate font-mono">{where}</span>
          <ChevronDown aria-hidden className="size-3 shrink-0 text-muted-foreground" />
        </button>
      }
    >
      {({ close }) => (
        <Places
          projects={projects}
          place={place}
          onPick={(p) => {
            close();
            onPick(p);
          }}
        />
      )}
    </Popover>
  );
}
