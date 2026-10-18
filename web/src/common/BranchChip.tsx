import { GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Chip } from '@ui/Chip';
import { MenuButton, type MenuEntry } from '@ui/Menu';
import { branchUrl, compareUrl } from '../projects/github';

const open = (url: string) => window.open(url, '_blank', 'noopener,noreferrer');

export interface BranchChipProps {
  branch: string;
  /** `owner/repo` when the project is on GitHub: the chip then opens a menu with GitHub links. */
  repo?: string | null;
  /** The project's base branch: adds "Open PR" (compare `base...branch`) unless this is the base branch. */
  base?: string;
  className?: string;
}

/** A branch name in a mono chip; on GitHub projects a menu: Open on GitHub (tree), Open PR (compare), Copy name. */
export function BranchChip({ branch, repo, base, className }: BranchChipProps) {
  const chip = (
    <Chip icon={<GitBranch aria-hidden />} title={branch} className={repo ? undefined : className}>
      {branch}
    </Chip>
  );
  if (!repo) return chip;
  const entries = (): MenuEntry[] => [
    { label: 'Open on GitHub', onSelect: () => open(branchUrl(repo, branch)) },
    ...(base && base !== branch ? [{ label: 'Open PR', onSelect: () => open(compareUrl(repo, base, branch)) }] : []),
    { label: 'Copy branch name', onSelect: () => void navigator.clipboard?.writeText(branch).catch(() => {}) },
  ];
  return (
    <MenuButton label={`Branch ${branch}`} entries={entries} placement="bottom start">
      <button
        type="button"
        aria-label={`Branch ${branch}`}
        className={cn('inline-flex max-w-full min-w-0 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring hover:[&>span]:bg-accent', className)}
      >
        {chip}
      </button>
    </MenuButton>
  );
}

/** A project's key (`PDT`) in a mono chip. */
export function ProjectKey({ projectKey, className }: { projectKey: string; className?: string }) {
  return (
    <Chip title="Project key" className={className}>
      {projectKey}
    </Chip>
  );
}
