import type { Activity } from '../api/gen/Activity';

const LABEL: Record<Activity, string> = { working: 'Working', needs_you: 'Needs you', idle: 'Idle', unknown: 'Unknown' };
const TONE: Record<Activity, string> = {
  working: 'text-running',
  needs_you: 'text-waiting font-semibold',
  idle: 'text-muted-foreground',
  unknown: 'text-muted-foreground italic',
};

/** An agent's activity; `unseen` adds an accent dot (it became idle or needs you since anyone last looked). */
export function ActivityBadge({ activity, unseen }: { activity: Activity | null; unseen: boolean }) {
  if (!activity) return null;
  return (
    <span className={`ml-auto flex items-center gap-1 text-meta ${TONE[activity]}`}>
      {unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-primary" />}
      {LABEL[activity]}
    </span>
  );
}
