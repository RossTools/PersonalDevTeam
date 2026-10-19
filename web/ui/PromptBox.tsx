import { useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { ArrowUp, Square } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { IconButton } from './IconButton';
import { Textarea } from './Textarea';

export interface PromptBoxProps {
  value: string;
  onChange: (text: string) => void;
  /** Called with Enter's modifier state; the box itself handles nothing but the buttons. */
  onKeyDown?: (e: KeyboardEvent<HTMLTextAreaElement>) => void;
  /** The send button (shown while there is text, or always when `onStop` is absent). */
  onSend: () => void;
  /** Shown as a Stop button (square) while set, e.g. while the agent is working. */
  onStop?: () => void;
  /** Accessible name of the text box. */
  label: string;
  placeholder?: string;
  /** Names the send button (default "Send"); e.g. "Queue" while working. */
  sendLabel?: string;
  sendShortcut?: ReactNode;
  stopLabel?: string;
  /** Sending: the send button shows it is busy. */
  pending?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  /** Left of the bottom row: small controls (pickers, chips). */
  toolbar?: ReactNode;
  /** Right of the bottom row, before the buttons: quiet text (a readout, a key hint). */
  meta?: ReactNode;
  /** Over the box, e.g. completions for what is being typed (`SuggestionList`). */
  above?: ReactNode;
  /** Inside the box, over the text: the attachments so far (`ImageThumb`s). */
  attachments?: ReactNode;
  /** Files pasted or dropped onto the box (the paste's text is kept when it has no files). */
  onFiles?: (files: File[]) => void;
  /** The send button is enabled (default: there is text). E.g. an image alone may be sent. */
  canSend?: boolean;
  /** The text area, for the caret position. */
  inputRef?: RefObject<HTMLTextAreaElement | null>;
  /** While `above` lists completions: the listbox id and the highlighted option's id. */
  completion?: { listId: string; activeId?: string };
  className?: string;
}

/**
 * A chat prompt box: one rounded box with a text area that grows with its content (up to about ten lines), and a
 * bottom row of controls with a round send button, or a stop button while something runs.
 */
export function PromptBox({
  value,
  onChange,
  onKeyDown,
  onSend,
  onStop,
  label,
  placeholder,
  sendLabel = 'Send',
  sendShortcut,
  stopLabel = 'Stop',
  pending = false,
  disabled = false,
  autoFocus = false,
  toolbar,
  meta,
  above,
  attachments,
  onFiles,
  canSend,
  inputRef,
  completion,
  className,
}: PromptBoxProps) {
  const own = useRef<HTMLTextAreaElement>(null);
  const ref = inputRef ?? own;
  const [dragging, setDragging] = useState(false);
  const empty = !value.trim();
  const sendable = canSend ?? !empty;
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...e.clipboardData.files];
    if (onFiles && files.length) {
      e.preventDefault();
      onFiles(files);
    }
  };
  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);
  const showSend = !onStop || sendable;
  return (
    <div className={cn('relative', className)}>
    {above && <div className="absolute inset-x-0 bottom-full z-(--pdt-z-popover) mb-1">{above}</div>}
    <div
      data-slot="prompt-box"
      data-dragging={dragging || undefined}
      className={cn(
        'flex flex-col rounded-xl border border-input bg-card shadow-sm transition-colors focus-within:border-primary focus-within:ring-2 focus-within:ring-ring motion-reduce:transition-none',
        dragging && 'border-primary ring-2 ring-ring',
        disabled && 'opacity-60',
      )}
      onDragOver={(e) => {
        if (!onFiles || !e.dataTransfer.types.includes('Files')) return;
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(e) => {
        if (!onFiles) return;
        e.preventDefault();
        setDragging(false);
        const files = [...e.dataTransfer.files];
        if (files.length) onFiles(files);
      }}
      // A click on the box's padding focuses the text, as if the whole box were the field.
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          e.preventDefault();
          ref.current?.focus();
        }
      }}
    >
      {attachments && <div className="flex flex-wrap gap-1.5 px-3 pt-2.5">{attachments}</div>}
      <Textarea
        ref={ref}
        aria-label={label}
        aria-autocomplete={completion ? 'list' : undefined}
        aria-controls={completion?.listId}
        aria-activedescendant={completion?.activeId}
        aria-expanded={completion ? true : undefined}
        onPaste={onPaste}
        rows={1}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        className="max-h-60 min-h-11 resize-none overflow-y-auto rounded-xl border-0 bg-transparent px-3 pt-2.5 pb-1 shadow-none focus-visible:border-0 focus-visible:ring-0"
      />
      <div className="flex min-w-0 items-center gap-1 px-2 pb-2">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">{toolbar}</div>
        {meta && <div className="hidden shrink-0 text-meta text-muted-foreground tabular-nums sm:block">{meta}</div>}
        {onStop && (
          <IconButton label={stopLabel} variant="secondary" onClick={onStop} className="rounded-full">
            <Square aria-hidden className="size-3 fill-current" />
          </IconButton>
        )}
        {showSend && (
          <IconButton
            label={sendLabel}
            shortcut={sendShortcut}
            variant="primary"
            pending={pending}
            disabled={!sendable || disabled}
            onClick={onSend}
            className="rounded-full"
          >
            <ArrowUp aria-hidden className="size-4" />
          </IconButton>
        )}
      </div>
    </div>
    </div>
  );
}
