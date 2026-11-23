import { Button } from '@ui';
import type { TicketStage } from '../api/gen/TicketStage';
import { allStages, BACKLOG_STAGES, OPEN_STAGES, sameStages, STAGES } from './model';

const PRESETS: { label: string; stages: TicketStage[] }[] = [
  { label: 'Backlog (Draft, Ready)', stages: BACKLOG_STAGES },
  { label: 'All open', stages: OPEN_STAGES },
  { label: 'Show closed', stages: STAGES },
];

/** The stage filter's one-click picks, under its checkboxes: the backlog, every open stage, or every stage. */
export function StagePresets({ stages, onChange }: { stages: TicketStage[]; onChange: (stages: TicketStage[]) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-0.5">
      {PRESETS.map((p) => {
        const on = p.stages === STAGES ? allStages(stages) : sameStages(stages, p.stages);
        return (
          <Button key={p.label} variant={on ? 'secondary' : 'ghost'} size="sm" compact aria-pressed={on} onClick={() => onChange(p.stages)}>
            {p.label}
          </Button>
        );
      })}
    </div>
  );
}
