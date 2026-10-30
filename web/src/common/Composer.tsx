import { useRef, useState, type ComponentProps, type KeyboardEvent, type ReactNode } from 'react';
import { Paperclip } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Alert } from '@ui/Alert';
import { Button } from '@ui/Button';
import { IconButton } from '@ui/IconButton';
import { Textarea } from '@ui/Textarea';
import { actionError } from './useAction';
import { insertAtCaret } from './insertText';

export interface ComposerProps {
  /** Sends the text (untrimmed). On success the box clears; on a rejection the text stays and the error shows. */
  onSend: (text: string) => Promise<unknown> | void;
  /** Escape, and the Close button (shown only when this is given). */
  onCancel?: () => void;
  /**
   * `enter` (default): Enter sends, Shift+Enter adds a line. `mod-enter`: Enter adds a line, Cmd/Ctrl+Enter sends.
   * Cmd/Ctrl+Enter sends in both.
   */
  submitOn?: 'enter' | 'mod-enter';
  /** Accessible name of the text box (default "Prompt"). */
  label?: string;
  placeholder?: string;
  sendLabel?: string;
  cancelLabel?: string;
  rows?: number;
  mono?: boolean;
  autoFocus?: boolean;
  disabled?: boolean;
  /**
   * Files pasted, dropped or picked (with the Attach button this adds): uploads them and resolves with the text to put
   * at the caret for each (a Markdown image).
   */
  onFiles?: (files: File[]) => Promise<string[]>;
  /** What the Attach button's picker offers (`accept`). */
  accept?: string;
  /** Under the box: e.g. the uploads in flight. */
  status?: ReactNode;
  className?: string;
}

/** A multi-line prompt box with Send: the terminal's Compose now; conversation, review feedback and comments later. */
export function Composer({
  onSend,
  onCancel,
  submitOn = 'enter',
  label = 'Prompt',
  placeholder,
  sendLabel = 'Send',
  cancelLabel = 'Close',
  rows = 3,
  mono = false,
  autoFocus = true,
  disabled = false,
  onFiles,
  accept,
  status,
  className,
}: ComposerProps) {
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const attach = async (files: File[]) => {
    if (!onFiles || files.length === 0) return;
    const snippets = await onFiles(files);
    if (snippets.length) setText((t) => insertAtCaret(t, box.current, snippets.join('\n')));
  };
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);
  const empty = !text.trim();

  const send = async () => {
    if (empty || disabled || sending.current) return;
    sending.current = true;
    setPending(true);
    setError(null);
    try {
      await onSend(text);
      setText('');
    } catch (err) {
      setError(actionError(err));
    } finally {
      sending.current = false;
      setPending(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape' && onCancel) {
      e.preventDefault();
      onCancel();
      return;
    }
    if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
    const mod = e.metaKey || e.ctrlKey;
    if (mod || (submitOn === 'enter' && !e.shiftKey && !e.altKey)) {
      e.preventDefault();
      void send();
    }
  };

  const hint = submitOn === 'enter' ? 'Enter sends; Shift+Enter adds a line.' : 'Enter adds a line; ⌘/Ctrl+Enter sends.';
  return (
    <div className={cn('flex shrink-0 flex-col gap-2 border-t border-border bg-card p-2', className)}>
      {error && <Alert tone="error">{error}</Alert>}
      <div className="flex items-end gap-2">
        <Textarea
          ref={box}
          aria-label={label}
          autoFocus={autoFocus}
          rows={rows}
          value={text}
          mono={mono}
          disabled={disabled}
          placeholder={placeholder ?? `Write a prompt. ${hint}`}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          {...(onFiles ? {
            onPaste: (e) => {
              const files = [...e.clipboardData.files];
              if (files.length) {
                e.preventDefault();
                void attach(files);
              }
            },
            onDragOver: (e) => {
              if (!e.dataTransfer.types.includes('Files')) return;
              e.preventDefault();
              setDragging(true);
            },
            onDragLeave: () => setDragging(false),
            onDrop: (e) => {
              const files = [...e.dataTransfer.files];
              setDragging(false);
              if (files.length) {
                e.preventDefault();
                void attach(files);
              }
            },
          } satisfies Partial<ComponentProps<'textarea'>> : {})}
          className={cn('min-h-16 flex-1 resize-y', dragging && 'border-primary ring-2 ring-ring')}
        />
        <div className="flex flex-col gap-1">
          {onFiles && (
            <>
              <input
                ref={picker}
                type="file"
                accept={accept}
                multiple
                hidden
                aria-hidden
                tabIndex={-1}
                onChange={(e) => {
                  void attach([...(e.target.files ?? [])]);
                  e.target.value = '';
                }}
              />
              <IconButton label="Attach files" variant="ghost" className="self-end" onClick={() => picker.current?.click()}>
                <Paperclip aria-hidden />
              </IconButton>
            </>
          )}
          <Button variant="primary" pending={pending} disabled={empty || disabled} onClick={() => void send()}>
            {sendLabel}
          </Button>
          {onCancel && (
            <Button variant="ghost" onClick={onCancel}>
              {cancelLabel}
            </Button>
          )}
        </div>
      </div>
      {status}
    </div>
  );
}
