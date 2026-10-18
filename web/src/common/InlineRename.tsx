import { useRef, useState } from 'react';
import { Input } from '@ui/Input';

export interface InlineRenameProps {
  /** Accessible name ("Tab name", "Workspace name", "New workspace name"). */
  label: string;
  /** The current name ('' for a new one). */
  initial?: string;
  placeholder?: string;
  maxLength?: number;
  /** Called with the trimmed name on Enter or blur, when it is non-empty and differs from `initial`. */
  onCommit: (value: string) => void;
  /** Called on Escape, or when Enter/blur leaves nothing to save (empty or unchanged). */
  onCancel: () => void;
  size?: 'sm' | 'md';
  className?: string;
}

/**
 * A text field that edits a name in place: focused with its text selected; Enter or blur commits, Escape cancels.
 * Finishes once (blur after Enter does nothing).
 */
export function InlineRename({ label, initial = '', placeholder, maxLength = 100, onCommit, onCancel, size = 'sm', className }: InlineRenameProps) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (save: boolean) => {
    if (done.current) return;
    done.current = true;
    const v = value.trim();
    if (save && v && v !== initial) onCommit(v);
    else onCancel();
  };
  return (
    <Input
      autoFocus
      aria-label={label}
      value={value}
      placeholder={placeholder}
      maxLength={maxLength}
      autoComplete="off"
      size={size}
      onChange={(e) => setValue(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          finish(true);
        } else if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          finish(false);
        }
      }}
      className={className}
    />
  );
}
