import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Check } from 'lucide-react';
import { Input, SettingsRow, Textarea, type InputProps, type SettingsRowProps } from '@ui';
import { cn } from '@ui/lib/utils';

/**
 * Settings save as they change (PDT-231): switches and selects at once, text on blur or Enter. Each control shows a
 * short "Saved" beside it, or its error.
 */
export type SaveState = { kind: 'idle' } | { kind: 'saving' } | { kind: 'saved' } | { kind: 'error'; message: string };

/** How long "Saved" shows. */
const SAVED_FOR_MS = 2000;

/** A control's save state, and `track` to follow a save (its promise) through it. */
export function useSaveState(): [SaveState, <T>(p: Promise<T>) => Promise<T | undefined>] {
  const [state, setState] = useState<SaveState>({ kind: 'idle' });
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const track = async <T,>(p: Promise<T>): Promise<T | undefined> => {
    clearTimeout(timer.current);
    setState({ kind: 'saving' });
    try {
      const v = await p;
      setState({ kind: 'saved' });
      timer.current = setTimeout(() => setState({ kind: 'idle' }), SAVED_FOR_MS);
      return v;
    } catch (e) {
      setState({ kind: 'error', message: e instanceof Error ? e.message : String(e) });
      return undefined;
    }
  };
  return [state, track];
}

/** "Saved ✓" for a moment after a save, or the error, beside the control. */
export function SaveStatus({ state }: { state: SaveState }) {
  if (state.kind === 'error') return <span role="alert" className="text-xs text-destructive">{state.message}</span>;
  if (state.kind === 'saved') {
    return (
      <span role="status" className="flex items-center gap-1 text-xs text-muted-foreground">
        <Check aria-hidden className="size-3" />Saved
      </span>
    );
  }
  return null;
}

export interface AutoSaveTextProps extends Omit<InputProps, 'value' | 'onChange' | 'onBlur' | 'onKeyDown' | 'defaultValue'> {
  /** The stored value: shown until the user types, and again after a save or Escape. */
  value: string;
  /** Saves the typed text (trimmed); a rejection shows its message. Called only when the text changed. */
  onSave: (text: string) => Promise<unknown>;
  /** A text area: saves on blur or Ctrl/⌘+Enter (Enter is a new line). */
  multiline?: boolean;
  rows?: number;
}

/**
 * A text field that saves on blur or Enter (PDT-231), showing "Saved" or the error beside it. Typed text stays while it
 * is being edited, however often the page re-renders with the stored value; Escape puts the stored value back.
 */
export function AutoSaveText({ value, onSave, multiline = false, rows, size, mono, className, ...props }: AutoSaveTextProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [state, track] = useSaveState();
  const shown = draft ?? value;
  const commit = async () => {
    if (draft === null) return;
    if (draft.trim() === value.trim()) {
      setDraft(null);
      return;
    }
    const ok = await track(onSave(draft.trim()).then(() => true));
    // A failed save keeps the text, so it can be fixed.
    if (ok) setDraft(null);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (e.key === 'Escape' && draft !== null) {
      e.preventDefault();
      setDraft(null);
    } else if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void commit();
    }
  };
  const common = { value: shown, mono, className: cn('max-w-full', className), onBlur: () => void commit(), onKeyDown };
  return (
    <>
      {/* An error beside the field (it stays until fixed); "Saved" inside its end, so the row never reflows. */}
      {state.kind === 'error' && <SaveStatus state={state} />}
      <span data-slot={multiline ? undefined : 'input'} className="relative inline-flex max-w-full min-w-0">
        {multiline ? (
          <Textarea
            id={props.id}
            aria-label={props['aria-label']}
            placeholder={props.placeholder}
            disabled={props.disabled}
            spellCheck={props.spellCheck}
            {...common}
            rows={rows}
            onChange={(e) => setDraft(e.target.value)}
          />
        ) : (
          <Input {...props} {...common} size={size} onChange={(e) => setDraft(e.target.value)} />
        )}
        {state.kind === 'saved' && (
          <span className={cn('pointer-events-none absolute right-1.5 flex rounded-sm bg-background px-1', multiline ? 'bottom-1.5' : 'inset-y-1.5 items-center')}>
            <SaveStatus state={state} />
          </span>
        )}
      </span>
    </>
  );
}

export interface AutoSaveRowProps extends Omit<SettingsRowProps, 'children'> {
  /** The control; `track` follows its save, so "Saved" or the error shows beside it. */
  children: (track: <T>(p: Promise<T>) => Promise<T | undefined>) => ReactNode;
}

/** A settings row whose control (a switch or select) saves at once, with its "Saved" or error beside it. */
export function AutoSaveRow({ children, ...row }: AutoSaveRowProps) {
  const [state, track] = useSaveState();
  return (
    <SettingsRow {...row}>
      <SaveStatus state={state} />
      {children(track)}
    </SettingsRow>
  );
}
