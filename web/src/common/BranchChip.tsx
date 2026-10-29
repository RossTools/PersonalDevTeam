import { GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Chip } from '@ui/Chip';
import { MenuButton, type MenuEntry } from '@ui/Menu';
import type { ForgeLinks } from '../api/gen/ForgeLinks';
import { branchUrl, compareUrl } from '../projects/forgeLinks';

const open = (url: string) => window.open(url, '_blank', 'noopener,noreferrer');

export interface BranchChipProps {
  branch: string;
  /** The project's repository on its forge: the chip then opens a menu with links to the forge. */
  forge?: ForgeLinks | null;
  /** The project's base branch: adds "Open PR" (compare `base...branch`) unless this is the base branch. */
  base?: string;
  /** A worktree's branch: the menu starts with "Review changes" (its changes tab). */
  onReview?: () => void;
  className?: string;
}

/**
 * A branch name in a mono chip. With a menu when there is something to offer: Review changes (a worktree's), and on
 * a forge's projects Open on <forge> (the branch's page), Open PR (compare) and Copy name.
 */
export function BranchChip({ branch, forge, base, onReview, className }: BranchChipProps) {
  const tree = forge ? branchUrl(forge, branch) : null;
  const compare = forge && base && base !== branch ? compareUrl(forge, base, branch) : null;
  const menu = !!tree || !!compare || !!onReview;
  const chip = (
    <Chip icon={<GitBranch aria-hidden />} title={branch} className={menu ? undefined : className}>
      {branch}
    </Chip>
  );
  if (!menu) return chip;
  const entries = (): MenuEntry[] => [
    ...(onReview ? [{ label: 'Review changes', onSelect: onReview }] : []),
    ...(tree ? [{ label: `Open on ${forge!.name}`, onSelect: () => open(tree) }] : []),
    ...(compare ? [{ label: 'Open PR', onSelect: () => open(compare) }] : []),
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
