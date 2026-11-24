import type { ReactNode } from 'react';
import { useQueries, useQuery } from '@tanstack/react-query';
import { Check, ChevronDown, FolderGit2, GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { CommandList, type CommandItem } from '@ui/CommandList';
import { Popover } from '@ui/Popover';
import type { Project } from '../api/gen/Project';
import { remoteInstancesQuery } from '../projects/queries';
import { worktreesQuery } from './worktrees';

/**
 * A checkout of a project: its own, one of its worktrees (`worktreeId`), or (`instanceId`, the Files view only) the
 * project's checkout on a joined remote instance.
 */
export type Place = { projectId: string; worktreeId: string | null; instanceId?: string | null };

const REPOSITORY = 'Repository';
/** A place as one string (for keys and remembered state): `<project>:<worktree>`, or `<project>:@<instance>`. */
export const placeKey = (p: Place) => `${p.projectId}:${p.instanceId ? `@${p.instanceId}` : p.worktreeId ?? ''}`;
function placeOf(id: string): Place {
  const i = id.indexOf(':');
  const rest = id.slice(i + 1);
  const projectId = id.slice(0, i);
  if (rest.startsWith('@')) return { projectId, worktreeId: null, instanceId: rest.slice(1) };
  return { projectId, worktreeId: rest || null };
}

/**
 * The list inside the popover: every project, each with its main checkout, (with `remote`) its checkout on each joined
 * instance, and its worktrees (missing ones left out).
 */
function Places({ projects, place, remote, onPick }: { projects: Project[]; place: Place; remote: boolean; onPick: (p: Place) => void }) {
  const worktrees = useQueries({
    queries: projects.map((p) => worktreesQuery(p.id)),
  });
  const instances = useQueries({
    queries: projects.map((p) => ({ ...remoteInstancesQuery(p.id), enabled: remote && p.kind === 'repo' })),
  });
  const current = placeKey(place);
  const icon = (id: string, fallback: ReactNode) => (id === current ? <Check aria-hidden /> : fallback);
  const items: CommandItem[] = projects.flatMap((p, i) => {
    const repoId = placeKey({ projectId: p.id, worktreeId: null });
    return [
      { id: repoId, label: REPOSITORY, group: p.name, keywords: [p.name, p.key, p.baseBranch], icon: icon(repoId, <FolderGit2 aria-hidden />) },
      ...(remote ? instances[i]?.data?.instances ?? [] : [])
        .filter((r) => r.state !== 'removed')
        .map((r) => {
          const id = placeKey({ projectId: p.id, worktreeId: null, instanceId: r.instanceId });
          return { id, label: REPOSITORY, detail: `on ${r.name}`, group: p.name, keywords: [p.name, p.key, r.name], icon: icon(id, <FolderGit2 aria-hidden />) };
        }),
      ...(worktrees[i]?.data ?? [])
        .filter((w) => !w.missing)
        .map((w) => {
          const id = placeKey({ projectId: p.id, worktreeId: w.id });
          return {
            id,
            label: w.branch,
            ...(w.instance ? { detail: `on ${w.instance.name}` } : {}),
            group: p.name,
            keywords: [p.name, p.key],
            icon: icon(id, <GitBranch aria-hidden />),
          };
        }),
    ];
  });
  return <CommandList label="Places" placeholder="Filter projects and branches…" items={items} onAction={(id) => onPick(placeOf(id))} />;
}

/** A place's project name and where in it: "Repository", "Repository · on <instance>" or a worktree's branch. */
export function usePlaceName(projects: Project[], place: Place) {
  const worktrees = useQuery({ ...worktreesQuery(place.projectId), enabled: !!place.projectId && !place.instanceId });
  const instances = useQuery({ ...remoteInstancesQuery(place.projectId), enabled: !!place.projectId && !!place.instanceId });
  const project = projects.find((p) => p.id === place.projectId);
  const worktree = place.worktreeId ? worktrees.data?.find((w) => w.id === place.worktreeId) : undefined;
  const instance = place.instanceId ? instances.data?.instances.find((r) => r.instanceId === place.instanceId) : undefined;
  const where = place.instanceId
    ? `${REPOSITORY} · on ${instance?.name ?? '…'}`
    : place.worktreeId
      ? worktree ? `${worktree.branch}${worktree.instance ? ` · on ${worktree.instance.name}` : ''}` : '…'
      : REPOSITORY;
  return { projectName: project?.name ?? 'No project', where };
}

/**
 * Picks a project and a place in it (the main checkout or one of its worktrees; with `remote`, also the project's
 * checkout on each joined instance): one chip "Project › place" that opens a filterable list. The trigger is named
 * "Place: <project> › <place>".
 */
export function PlacePicker({ projects, place, onPick, remote = false, className }: {
  projects: Project[];
  place: Place;
  onPick: (p: Place) => void;
  remote?: boolean;
  className?: string;
}) {
  const { projectName, where } = usePlaceName(projects, place);
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
          remote={remote}
          onPick={(p) => {
            close();
            onPick(p);
          }}
        />
      )}
    </Popover>
  );
}
