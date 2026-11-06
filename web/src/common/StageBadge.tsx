import { Badge } from '@ui';
import { stageLabel } from './StageStepper';

/** A ticket's stage as a badge: the running tone while it is in progress, neutral otherwise. */
export function StageBadge({ stage }: { stage: string }) {
  return <Badge tone={stage === 'in_progress' ? 'running' : 'neutral'}>{stageLabel(stage)}</Badge>;
}
