import type { ReactNode } from 'react';
import { X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Chip, IconButton, SuggestInput } from '@ui';
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

/** Chips with a remove button each, and a field that adds one: pick an existing one from the list, or type a new one
 *  and press Enter (or a comma). */
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
      <SuggestInput
        size="sm"
        aria-label={`Add ${lower}`}
        placeholder={`Add ${lower}…`}
        suggestions={offered}
        onCommit={add}
        disabled={disabled}
        maxLength={noun === 'Area' ? 60 : 30}
        commitKeys={[',']}
        className="w-32"
      />
    </div>
  );
}
