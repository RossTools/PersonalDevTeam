import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Paperclip } from 'lucide-react';
import { Alert, IconButton, ImageThumb, Kbd, PromptBox, Select, StatusDot, SuggestionList, suggestionId, type SelectOption, type Suggestion } from '@ui';
import { cn } from '@ui/lib/utils';
import type { AgentCommandInfo } from '../api/gen/AgentCommandInfo';
import type { AgentModel } from '../api/gen/AgentModel';
import type { Delivery } from '../api/gen/Delivery';
import type { PermissionMode } from '../api/gen/PermissionMode';
import { actionError } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { softKeyboardOpen } from '../common/viewport';
import { isMac } from '../palette/useMetaK';
import { loadDraft, saveDraft } from './drafts';
import { applyCompletion, commandSuggestions, completionAt, type LimitWarning } from './completion';

/** Image types the agents take (and PDT stores). */
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
const MAX_IMAGES = 10;

interface Draft {
  key: string;
  name: string;
  url: string;
  /** Set once uploaded. */
  id?: string;
  error?: string;
}

export const PERMISSION_LABEL: Record<PermissionMode, string> = {
  ask: 'Ask',
  accept_edits: 'Accept edits',
  plan: 'Plan',
  bypass: 'Bypass (dangerous)',
};
export const PERMISSION_MODES: PermissionMode[] = ['ask', 'accept_edits', 'plan', 'bypass'];

/** The model picker's value for "the agent's default" (`model: null`). */
const DEFAULT_MODEL = '__default__';

/** Ask / Accept edits / Plan / Bypass; Bypass shows in the destructive colour. */
export function PermissionModeSelect({ value, onChange, modes = PERMISSION_MODES, disabled, size = 'sm', quiet = false, className, label = 'Permission mode' }: {
  value: PermissionMode;
  onChange: (m: PermissionMode) => void;
  modes?: PermissionMode[];
  disabled?: boolean;
  size?: 'sm' | 'md';
  /** A borderless chip (the prompt box's toolbar). */
  quiet?: boolean;
  className?: string;
  label?: string;
}) {
  const list = modes.length ? modes : PERMISSION_MODES;
  return (
    <Select<PermissionMode>
      aria-label={label}
      size={size}
      quiet={quiet}
      disabled={disabled}
      value={value}
      onChange={onChange}
      options={(list.includes(value) ? list : [...list, value]).map((m) => ({ value: m, label: PERMISSION_LABEL[m] }))}
      className={cn(
        value === 'bypass' && (quiet ? '[&_[data-slot=select-trigger]]:text-destructive' : '[&_[data-slot=select-trigger]]:border-destructive [&_[data-slot=select-trigger]]:text-destructive'),
        className,
      )}
    />
  );
}

export interface ConversationComposerProps {
  turnRunning: boolean;
  /** `images`: attachment ids from `onUpload`. */
  onSend: (text: string, delivery?: Delivery, images?: string[]) => Promise<unknown>;
  /** The agent's commands for `/` (Claude expands them; Codex runs the ones PDT maps). */
  commands?: AgentCommandInfo[];
  /** Commands that work only in the agent's TUI: listed, not pickable. */
  terminalCommands?: string[];
  /** Files for `@`, best match first. */
  onSearchFiles?: (query: string) => Promise<string[]>;
  /** Stores an image for the next message; enables paste, drop and the attach button. */
  onUpload?: (file: File) => Promise<{ id: string }>;
  /** A rate-limit warning, shown over the box (also on phones). */
  limit?: LimitWarning | null;
  onInterrupt: () => void;
  /** The agent stopped unexpectedly (state.stopped). */
  stopped?: string | null;
  agentName?: string;
  models: AgentModel[];
  /** null: the agent's default. */
  model: string | null;
  onModel: (m: string | null) => void;
  permissionModes: PermissionMode[];
  permissionMode: PermissionMode;
  onPermissionMode: (m: PermissionMode) => void;
  /** A small readout in the bottom row (context used). */
  readout?: string | null;
  autoFocus?: boolean;
  /** Keeps unsent text for this session across unmounts (clicking away and back). */
  draftKey?: string;
}

/**
 * Whether Enter adds a new line instead of sending: on a phone typing on its on-screen keyboard, where ⬆ sends. A
 * hardware keyboard (no on-screen one) keeps Enter to send.
 */
export const enterAddsLine = (phone: boolean) => phone && softKeyboardOpen();

/**
 * The conversation's prompt box. Enter sends (Shift+Enter adds a line); while the agent works Enter queues for its
 * next opportunity and Cmd/Ctrl+Enter interrupts and sends now; Escape or Stop interrupts. The bottom row has the
 * model and permission-mode pickers. On a phone's on-screen keyboard Enter adds a line and ⬆ sends (`enterAddsLine`);
 * ⬆ turns into Stop while the agent works and the box is empty.
 */
export function ConversationComposer({
  turnRunning,
  onSend,
  onInterrupt,
  stopped,
  agentName = 'Claude',
  models,
  model,
  onModel,
  permissionModes,
  permissionMode,
  onPermissionMode,
  readout,
  autoFocus = false,
  commands = [],
  terminalCommands = [],
  onSearchFiles,
  onUpload,
  limit,
  draftKey,
}: ConversationComposerProps) {
  const phone = useMediaQuery(PHONE_QUERY);
  const [text, setText] = useState(() => (draftKey ? loadDraft(draftKey) : ''));
  useEffect(() => {
    if (draftKey) saveDraft(draftKey, text);
  }, [draftKey, text]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const filePicker = useRef<HTMLInputElement>(null);
  const listId = useId();

  // ---- `/` and `@` completion ----
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  // Escape closes the list for the word it was open on.
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [files, setFiles] = useState<{ query: string; paths: string[] } | null>(null);
  const placeCaret = useRef<number | null>(null);
  const completion = completionAt(text, Math.min(caret, text.length));
  const wordKey = completion ? `${completion.kind}:${completion.start}` : null;
  const fileQuery = completion?.kind === 'file' && onSearchFiles ? completion.query : null;
  useEffect(() => {
    if (fileQuery === null || !onSearchFiles) return;
    let stale = false;
    const t = setTimeout(() => {
      onSearchFiles(fileQuery).then(
        (paths) => !stale && setFiles({ query: fileQuery, paths }),
        () => !stale && setFiles({ query: fileQuery, paths: [] }),
      );
    }, 120);
    return () => {
      stale = true;
      clearTimeout(t);
    };
  }, [fileQuery, onSearchFiles]);
  const items: Suggestion[] =
    completion?.kind === 'command'
      ? commandSuggestions(commands, terminalCommands, completion.query)
      : completion?.kind === 'file' && files
        ? files.paths.map((p) => ({ id: p, label: p }))
        : [];
  const open =
    !!completion &&
    wordKey !== dismissed &&
    (completion.kind === 'command' ? commands.length + terminalCommands.length > 0 : !!onSearchFiles && files !== null);
  const query = completion?.query;
  useEffect(() => setActive(0), [query, completion?.kind]);
  const at = Math.min(active, Math.max(items.length - 1, 0));

  const syncCaret = () => setCaret(input.current?.selectionStart ?? 0);
  useEffect(() => {
    const onSel = () => {
      if (document.activeElement === input.current) syncCaret();
    };
    document.addEventListener('selectionchange', onSel);
    return () => document.removeEventListener('selectionchange', onSel);
  }, []);
  useLayoutEffect(() => {
    if (placeCaret.current === null || !input.current) return;
    const p = placeCaret.current;
    placeCaret.current = null;
    input.current.focus();
    input.current.setSelectionRange(p, p);
    setCaret(p);
  });
  const pick = (i: number) => {
    const it = items[i];
    if (!it || it.disabled || !completion) return;
    const insert = completion.kind === 'command' ? `/${it.id}` : `@${it.id}`;
    const next = applyCompletion(text, caret, completion, insert);
    setText(next.text);
    placeCaret.current = next.caret;
    setDismissed(null);
  };

  // ---- images ----
  const [drafts, setDrafts] = useState<Draft[]>([]);
  const draftsRef = useRef(drafts);
  draftsRef.current = drafts;
  useEffect(() => () => draftsRef.current.forEach((d) => URL.revokeObjectURL(d.url)), []);
  const addFiles = (list: File[]) => {
    if (!onUpload) return;
    const room = MAX_IMAGES - drafts.length;
    for (const [i, f] of list.entries()) {
      const key = `${Date.now()}-${i}-${f.name}`;
      const why = !IMAGE_TYPES.includes(f.type)
        ? 'Only PNG, JPEG, GIF and WebP images can be attached'
        : f.size > MAX_IMAGE_BYTES
          ? 'Images can be at most 10 MB'
          : i >= room
            ? `At most ${MAX_IMAGES} images per message`
            : null;
      if (why) {
        setError(why);
        continue;
      }
      const draft: Draft = { key, name: f.name || 'Pasted image', url: URL.createObjectURL(f) };
      setDrafts((d) => [...d, draft]);
      onUpload(f).then(
        (a) => setDrafts((d) => d.map((x) => (x.key === key ? { ...x, id: a.id } : x))),
        (err) => setDrafts((d) => d.map((x) => (x.key === key ? { ...x, error: actionError(err) } : x))),
      );
    }
  };
  const removeDraft = (key: string) =>
    setDrafts((d) => {
      const gone = d.find((x) => x.key === key);
      if (gone) URL.revokeObjectURL(gone.url);
      return d.filter((x) => x.key !== key);
    });
  const uploading = drafts.some((d) => !d.id && !d.error);
  const broken = drafts.some((d) => d.error);
  const canSend = (!!text.trim() || drafts.length > 0) && !uploading && !broken;

  const modelOptions: SelectOption[] = [
    { value: DEFAULT_MODEL, label: 'Default model' },
    ...models.filter((m) => m.value !== 'default').map((m) => ({ value: m.value, label: m.displayName })),
  ];
  if (model && !modelOptions.some((o) => o.value === model)) modelOptions.push({ value: model, label: model });

  const send = async (now = false) => {
    if (!canSend || sending.current) return;
    sending.current = true;
    setPending(true);
    setError(null);
    const images = drafts.flatMap((d) => (d.id ? [d.id] : []));
    try {
      const delivery = turnRunning ? (now ? 'now' : 'next') : undefined;
      await (images.length ? onSend(text, delivery, images) : onSend(text, delivery));
      setText('');
      drafts.forEach((d) => URL.revokeObjectURL(d.url));
      setDrafts([]);
    } catch (err) {
      setError(actionError(err));
    } finally {
      sending.current = false;
      setPending(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (open && !e.nativeEvent.isComposing) {
      if (e.key === 'Escape') {
        // Closes the list; it never interrupts the agent.
        e.preventDefault();
        e.stopPropagation();
        setDismissed(wordKey);
        return;
      }
      if (items.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
        e.preventDefault();
        setActive((a) => (a + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length);
        return;
      }
      if (items.length && (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey))) {
        e.preventDefault();
        pick(at);
        return;
      }
    }
    if (e.key === 'Escape' && turnRunning) {
      e.preventDefault();
      onInterrupt();
      return;
    }
    if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
    const mod = e.metaKey || e.ctrlKey;
    if (!mod && enterAddsLine(phone)) return;
    if (mod || (!e.shiftKey && !e.altKey)) {
      e.preventDefault();
      void send(mod);
    }
  };

  return (
    <div className="flex shrink-0 flex-col gap-2 px-3 pt-1 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
      {stopped && (
        <Alert tone="error" title={`${agentName} stopped`}>
          <span className="whitespace-pre-wrap">{stopped}</span>
          <span className="mt-1 block">Send a message to start it again.</span>
        </Alert>
      )}
      {error && <Alert tone="error">{error}</Alert>}
      {limit && (
        <p role="status" className={cn('mx-auto w-full max-w-3xl px-1 text-meta', limit.reached ? 'text-destructive' : 'text-waiting')}>
          {limit.text}
          {limit.reached ? '. Nothing is sent until you send again.' : ''}
        </p>
      )}
      {onUpload && (
        <input
          ref={filePicker}
          type="file"
          accept={IMAGE_TYPES.join(',')}
          multiple
          hidden
          aria-hidden
          tabIndex={-1}
          onChange={(e) => {
            addFiles([...(e.target.files ?? [])]);
            e.target.value = '';
          }}
        />
      )}
      <PromptBox
        label="Message"
        value={text}
        onChange={(t) => {
          setText(t);
          syncCaret();
          setError(null);
        }}
        inputRef={input}
        onKeyDown={onKeyDown}
        onFiles={onUpload ? addFiles : undefined}
        canSend={canSend}
        completion={open && items.length ? { listId, activeId: suggestionId(listId, at) } : undefined}
        above={
          open ? (
            <SuggestionList
              id={listId}
              label={completion?.kind === 'command' ? 'Commands' : 'Files'}
              items={items}
              active={at}
              onPick={pick}
              onHover={setActive}
              empty={completion?.kind === 'command' ? 'No matching commands' : 'No matching files'}
            />
          ) : undefined
        }
        attachments={
          drafts.length
            ? drafts.map((d) => (
                <ImageThumb key={d.key} src={d.url} alt={d.name} size="sm" pending={!d.id && !d.error} error={d.error} onRemove={() => removeDraft(d.key)} />
              ))
            : undefined
        }
        onSend={() => void send()}
        onStop={turnRunning ? onInterrupt : undefined}
        stopLabel="Stop (Esc)"
        sendLabel={turnRunning ? 'Queue' : 'Send'}
        pending={pending}
        autoFocus={autoFocus}
        placeholder={turnRunning ? `${agentName} is working. Queue a message…` : `Message ${agentName}…`}
        className="mx-auto w-full max-w-3xl"
        layout={phone ? 'phone' : 'default'}
        toolbar={
          <>
            {onUpload && (
              <IconButton label="Attach images" size="sm" variant="ghost" onClick={() => filePicker.current?.click()} className="rounded-full">
                <Paperclip aria-hidden className="size-3.5" />
              </IconButton>
            )}
            <Select
              aria-label="Model"
              size="sm"
              quiet
              value={model ?? DEFAULT_MODEL}
              onChange={(v) => onModel(v === DEFAULT_MODEL ? null : v)}
              options={modelOptions}
            />
            <PermissionModeSelect value={permissionMode} onChange={onPermissionMode} modes={permissionModes} quiet />
          </>
        }
        meta={
          phone ? (
            <span className="flex items-center gap-1.5" role="status">
              <span aria-hidden className="flex"><StatusDot status={turnRunning ? 'running' : stopped ? 'failed' : 'idle'} pulse /></span>
              {turnRunning ? 'working' : stopped ? 'stopped' : 'ready'}
            </span>
          ) : turnRunning ? (
            <span>
              <Kbd>↵</Kbd> queues · <Kbd>{isMac() ? '⌘↵' : 'Ctrl+↵'}</Kbd> sends now · <Kbd>Esc</Kbd> stops
            </span>
          ) : (
            readout
          )
        }
      />
    </div>
  );
}
