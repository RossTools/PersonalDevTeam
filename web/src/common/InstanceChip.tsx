import { Server } from 'lucide-react';
import { Chip, relativeTime } from '@ui';
import type { InstanceRef } from '../api/gen/InstanceRef';

/** How long without a heartbeat before an instance counts as offline (the owner's rule). */
const OFFLINE_AFTER_MS = 90_000;

/**
 * "on win-box": a session or worktree that runs on another instance of PDT (a remote project's), with "offline" while
 * the owner hasn't heard from it lately and when it last did. Nothing when local.
 */
export function InstanceChip({ instance, className }: { instance: InstanceRef | null | undefined; className?: string }) {
  if (!instance) return null;
  const seen = instance.lastSeenAt;
  const offline = seen == null || Date.now() - seen > OFFLINE_AFTER_MS;
  const when = seen == null ? 'not heard from yet' : `last seen ${relativeTime(seen)}`;
  return (
    <Chip
      icon={<Server aria-hidden />}
      title={offline ? `${instance.name} is offline · ${when}` : `Runs on ${instance.name} · ${when}`}
      className={className}
    >
      on {instance.name}{offline && ' · offline'}
    </Chip>
  );
}
