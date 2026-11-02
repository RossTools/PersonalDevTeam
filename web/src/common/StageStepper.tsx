import { Check, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import type { TicketStage } from '../api/gen/TicketStage';

export type { TicketStage };

export const STAGE_LABEL: Record<TicketStage, string> = {
  draft: 'Draft',
  ready: 'Ready',
  todo: 'To do',
  in_progress: 'In progress',
  done: 'Done',
  wont_do: "Won't do",
};

const FLOW: TicketStage[] = ['draft', 'ready', 'todo', 'in_progress', 'done'];

export interface StageStepperProps {
  stage: TicketStage;
  /** Dots only, with just the current stage's label (the peek sheet, narrow headers). */
  compact?: boolean;
  className?: string;
}

/**
 * Draft → Ready → To do → In progress → Done, the current stage marked (`aria-current="step"`), earlier ones checked. A
 * ticket closed as Won't do shows that as a crossed-out final step in place of Done.
 */
export function StageStepper({ stage, compact = false, className }: StageStepperProps) {
  const closedUndone = stage === 'wont_do';
  const steps: TicketStage[] = closedUndone ? ['draft', 'ready', 'todo', 'in_progress', 'wont_do'] : FLOW;
  const at = closedUndone ? steps.length - 1 : FLOW.indexOf(stage);
  return (
    <ol aria-label="Stage" className={cn('flex min-w-0 items-center', compact ? 'gap-1' : 'gap-1.5', className)}>
      {steps.map((s, i) => {
        const current = i === at;
        // Done is the end of the flow: shown checked like the steps before it.
        const past = (i < at && !closedUndone) || (current && s === 'done');
        const crossed = s === 'wont_do';
        const label = STAGE_LABEL[s];
        return (
          <li
            key={s}
            aria-current={current ? 'step' : undefined}
            data-state={current ? 'current' : past ? 'complete' : 'upcoming'}
            className="flex min-w-0 items-center gap-1.5"
          >
            {i > 0 && (
              <span aria-hidden className={cn('h-px shrink-0 bg-border', compact ? 'w-2' : 'w-3 sm:w-5', past || (current && !crossed) ? 'bg-primary' : undefined)} />
            )}
            <span
              aria-hidden
              className={cn(
                'flex shrink-0 items-center justify-center rounded-full border [&>svg]:size-2.5',
                compact ? 'size-3' : 'size-4',
                past && 'border-primary bg-primary text-primary-foreground',
                current && !past && !crossed && 'border-primary bg-accent text-link ring-2 ring-ring',
                current && crossed && 'border-muted-foreground bg-muted text-muted-foreground',
                !past && !current && 'border-border bg-card',
              )}
            >
              {past && !compact && <Check strokeWidth={3} />}
              {crossed && <X strokeWidth={3} />}
              {current && !past && !crossed && <span className="size-1.5 rounded-full bg-primary" />}
            </span>
            <span
              className={cn(
                'truncate text-xs',
                compact && !current && 'sr-only',
                current ? 'font-medium text-foreground' : 'text-muted-foreground',
                crossed && 'line-through',
              )}
            >
              {label}
            </span>
          </li>
        );
      })}
    </ol>
  );
}
