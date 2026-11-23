import type { ReactNode } from 'react';
import { Plus, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, Chip, IconButton, Popover, SuggestInput } from '@ui';
import { normaliseArea, normaliseLabel } from './model';

export interface LabelsEditorProps {
  labels: string[];
  /** The whole new set (sorted). */
  onChange: (labels: string[]) => void;
  /** Offered while typing (the project's labels). */
  suggestions?: string[];
  /** Labels (the default: lower-cased, 30 characters) or areas (kept as written, 60): also names the controls. */
  noun?: 'Label' | 'Area';
  disabled?: boolean;
  /** How each chip looks (a plain Chip by default). */
  renderChip?: (label: string) => ReactNode;
  className?: string;
}

/** Chips with a remove button each, then a ghost "+ Add" that opens a field in a popover: pick an existing one from the
 *  list, or type a new one and press Enter (or a comma). The field stays open to add more; Escape closes it. */
export function LabelsEditor({ labels, onChange, suggestions = [], noun = 'Label', disabled, renderChip, className }: LabelsEditorProps) {
  const lower = noun.toLowerCase();
  const add = (text: string) => {
    const v = (noun === 'Area' ? normaliseArea : normaliseLabel)(text);
    if (v && !labels.includes(v)) onChange([...labels, v].sort());
  };
  const offered = suggestions.filter((s) => !labels.includes(s));
  return (
    <div className={cn('flex flex-wrap items-center gap-1', className)}>
      {labels.map((l) => (
        <span key={l} className="inline-flex items-center">
          {renderChip ? renderChip(l) : <Chip>{l}</Chip>}
          <IconButton
            label={`Remove ${lower} ${l}`}
            size="sm"
            noTooltip
            disabled={disabled}
            className="size-5"
            onClick={() => onChange(labels.filter((x) => x !== l))}
          >
            <X aria-hidden />
          </IconButton>
        </span>
      ))}
      <Popover
        label={`Add ${lower}`}
        placement="bottom start"
        className="w-56 p-1.5"
        trigger={
          <Button variant="ghost" size="sm" compact aria-label={`Add ${lower}`} disabled={disabled} className="text-muted-foreground">
            <Plus aria-hidden /> Add
          </Button>
        }
      >
        <SuggestInput
          size="sm"
          aria-label={`Add ${lower}`}
          placeholder={`${noun} name…`}
          suggestions={offered}
          onCommit={add}
          disabled={disabled}
          maxLength={noun === 'Area' ? 60 : 30}
          commitKeys={[',']}
          autoFocus
        />
      </Popover>
    </div>
  );
}
