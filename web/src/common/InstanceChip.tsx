import { Server } from 'lucide-react';
import { Chip } from '@ui';
import type { InstanceRef } from '../api/gen/InstanceRef';

/** "on win-box": a session or worktree that runs on another instance of PDT (a remote project's). Nothing when local. */
export function InstanceChip({ instance, className }: { instance: InstanceRef | null | undefined; className?: string }) {
  if (!instance) return null;
  return (
    <Chip icon={<Server aria-hidden />} title={`Runs on ${instance.name}`} className={className}>
      on {instance.name}
    </Chip>
  );
}
