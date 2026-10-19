import { useRef, useState, type KeyboardEvent } from 'react';
import { Alert, Kbd, PromptBox, Select, type SelectOption } from '@ui';
import { cn } from '@ui/lib/utils';
import type { AgentModel } from '../api/gen/AgentModel';
import type { Delivery } from '../api/gen/Delivery';
import type { PermissionMode } from '../api/gen/PermissionMode';
import { actionError } from '../common/useAction';
import { isMac } from '../palette/useMetaK';

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
  onSend: (text: string, delivery?: Delivery) => Promise<unknown>;
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
}

/**
 * The conversation's prompt box. Enter sends (Shift+Enter adds a line); while the agent works Enter queues for its
 * next opportunity and Cmd/Ctrl+Enter interrupts and sends now; Escape or Stop interrupts. The bottom row has the
 * model and permission-mode pickers.
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
}: ConversationComposerProps) {
  const [text, setText] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);

  const modelOptions: SelectOption[] = [
    { value: DEFAULT_MODEL, label: 'Default model' },
    ...models.filter((m) => m.value !== 'default').map((m) => ({ value: m.value, label: m.displayName })),
  ];
  if (model && !modelOptions.some((o) => o.value === model)) modelOptions.push({ value: model, label: model });

  const send = async (now = false) => {
    if (!text.trim() || sending.current) return;
    sending.current = true;
    setPending(true);
    setError(null);
    try {
      await onSend(text, turnRunning ? (now ? 'now' : 'next') : undefined);
      setText('');
    } catch (err) {
      setError(actionError(err));
    } finally {
      sending.current = false;
      setPending(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape' && turnRunning) {
      e.preventDefault();
      onInterrupt();
      return;
    }
    if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
    const mod = e.metaKey || e.ctrlKey;
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
      <PromptBox
        label="Message"
        value={text}
        onChange={setText}
        onKeyDown={onKeyDown}
        onSend={() => void send()}
        onStop={turnRunning ? onInterrupt : undefined}
        stopLabel="Stop (Esc)"
        sendLabel={turnRunning ? 'Queue' : 'Send'}
        pending={pending}
        autoFocus={autoFocus}
        placeholder={turnRunning ? `${agentName} is working. Queue a message…` : `Message ${agentName}…`}
        className="mx-auto w-full max-w-3xl"
        toolbar={
          <>
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
          turnRunning ? (
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
