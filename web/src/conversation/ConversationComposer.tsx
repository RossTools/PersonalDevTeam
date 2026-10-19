import { Square } from 'lucide-react';
import { Alert, Button, Select, type SelectOption } from '@ui';
import { cn } from '@ui/lib/utils';
import type { AgentModel } from '../api/gen/AgentModel';
import type { Delivery } from '../api/gen/Delivery';
import type { PermissionMode } from '../api/gen/PermissionMode';
import { Composer } from '../common/Composer';

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
export function PermissionModeSelect({ value, onChange, modes = PERMISSION_MODES, disabled, size = 'sm', className, label = 'Permission mode' }: {
  value: PermissionMode;
  onChange: (m: PermissionMode) => void;
  modes?: PermissionMode[];
  disabled?: boolean;
  size?: 'sm' | 'md';
  className?: string;
  label?: string;
}) {
  const list = modes.length ? modes : PERMISSION_MODES;
  return (
    <Select<PermissionMode>
      aria-label={label}
      size={size}
      disabled={disabled}
      value={value}
      onChange={onChange}
      options={(list.includes(value) ? list : [...list, value]).map((m) => ({ value: m, label: PERMISSION_LABEL[m] }))}
      className={cn(
        value === 'bypass' && '[&_[data-slot=select-trigger]]:border-destructive [&_[data-slot=select-trigger]]:text-destructive',
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
  /** A small readout at the end of the footer (context used). */
  readout?: string | null;
  autoFocus?: boolean;
}

/**
 * The conversation's prompt box. Enter sends; while the agent works Enter queues for its next opportunity and
 * Cmd/Ctrl+Enter interrupts and sends now; Escape or Stop interrupts. Footer: model and permission mode.
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
  const modelOptions: SelectOption[] = [
    { value: DEFAULT_MODEL, label: 'Default model' },
    ...models.filter((m) => m.value !== 'default').map((m) => ({ value: m.value, label: m.displayName })),
  ];
  if (model && !modelOptions.some((o) => o.value === model)) modelOptions.push({ value: model, label: model });

  return (
    <Composer
      label="Message"
      rows={2}
      autoFocus={autoFocus}
      sendLabel={turnRunning ? 'Queue' : 'Send'}
      placeholder={
        turnRunning
          ? `Enter queues a message for ${agentName}; ⌘/Ctrl+Enter interrupts and sends now. Esc stops.`
          : `Message ${agentName}. Enter sends; Shift+Enter adds a line.`
      }
      onSend={(text) => onSend(text, turnRunning ? 'next' : undefined)}
      onSendNow={(text) => onSend(text, turnRunning ? 'now' : undefined)}
      onEscape={turnRunning ? onInterrupt : undefined}
      className="pb-[max(0.5rem,env(safe-area-inset-bottom))]"
      above={
        stopped ? (
          <Alert tone="error" title={`${agentName} stopped`}>
            <span className="whitespace-pre-wrap">{stopped}</span>
            <span className="mt-1 block">Send a message to start it again.</span>
          </Alert>
        ) : undefined
      }
      actions={
        turnRunning ? (
          <Button variant="secondary" onClick={onInterrupt} title="Stop the current turn (Esc)">
            <Square aria-hidden className="size-3" /> Stop
          </Button>
        ) : undefined
      }
      footer={
        <div className="flex flex-wrap items-center gap-2">
          <Select
            aria-label="Model"
            size="sm"
            value={model ?? DEFAULT_MODEL}
            onChange={(v) => onModel(v === DEFAULT_MODEL ? null : v)}
            options={modelOptions}
          />
          <PermissionModeSelect value={permissionMode} onChange={onPermissionMode} modes={permissionModes} />
          {readout && <span className="ml-auto text-meta text-muted-foreground tabular-nums">{readout}</span>}
        </div>
      }
    />
  );
}
