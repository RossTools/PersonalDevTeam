import { useId, useState } from 'react';
import { X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Chip, IconButton, Input } from '@ui';
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
  className?: string;
}

/** Chips with a remove button each, and a field that adds one on Enter (suggesting the project's existing ones). */
export function LabelsEditor({ labels, onChange, suggestions = [], noun = 'Label', disabled, className }: LabelsEditorProps) {
  const [draft, setDraft] = useState('');
  const listId = useId();
  const lower = noun.toLowerCase();
  const add = () => {
    const v = (noun === 'Area' ? normaliseArea : normaliseLabel)(draft);
    setDraft('');
    if (v && !labels.includes(v)) onChange([...labels, v].sort());
  };
  const offered = suggestions.filter((s) => !labels.includes(s));
  return (
    <div className={cn('flex flex-wrap items-center gap-1', className)}>
      {labels.map((l) => (
        <span key={l} className="inline-flex items-center">
          <Chip>{l}</Chip>
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
      <Input
        size="sm"
        aria-label={`Add ${lower}`}
        placeholder={`Add ${lower}…`}
        list={offered.length > 0 ? listId : undefined}
        value={draft}
        disabled={disabled}
        maxLength={noun === 'Area' ? 60 : 30}
        autoComplete="off"
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ',') {
            e.preventDefault();
            add();
          }
        }}
        className="w-28"
      />
      {offered.length > 0 && (
        <datalist id={listId}>
          {offered.map((s) => <option key={s} value={s} />)}
        </datalist>
      )}
    </div>
  );
}
