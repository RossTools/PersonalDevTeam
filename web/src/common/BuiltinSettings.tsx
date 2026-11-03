import type { ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Link, Section, Select, Skeleton } from '@ui';
import type { BuiltinSettingsApi } from '../api/builtinSettings';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import type { NotificationChannel } from '../api/gen/NotificationChannel';
import { useConnections } from '../connections/queries';
import { routePath } from '../shell/route';
import { navigate } from '../shell/router';
import { AutoSaveRow } from './AutoSave';
import { invalidateWorkflow } from '../workflows/queries';

// Settings is the home of a behaviour PDT runs as a built-in workflow (PDT-384): plain controls that read and write
// the workflow's inputs, a "Customize as a workflow →" link, and, once the workflow is changed beyond its inputs or
// turned off, a notice naming it with Open and Reset to default. The workflow editor's side is
// workflows/BuiltinSettingsBanner.tsx.

export const openWorkflow = (id: string) => navigate(routePath({ view: 'workflows', target: { kind: 'workflow', id } }));

/** Saves some inputs; the view after it. */
export type PutInputs = (values: Record<string, unknown>) => Promise<BuiltinSettingsView>;

/**
 * A workflow Settings can't change: changed beyond its inputs (`follows`: "Updates follow"), or turned off (`off`:
 * what that means). Open goes to it; Reset to default puts back PDT's, on.
 */
export function CustomizedNotice({ name, workflowId, follows, off, onReset, resetting }: {
  name: string;
  workflowId: string;
  follows: string;
  /** Set when it is off: the notice says so. */
  off?: string;
  onReset: () => void;
  resetting: boolean;
}) {
  return (
    <Alert
      tone="info"
      action={
        <span className="flex gap-1">
          <Button size="sm" variant="secondary" onClick={() => openWorkflow(workflowId)}>Open</Button>
          <Button size="sm" variant="secondary" pending={resetting} onClick={onReset}>Reset to default</Button>
        </span>
      }
    >
      {off ? `The workflow ${name} is off: ${off}` : `${follows} your customized workflow ${name}.`}
    </Alert>
  );
}

/**
 * A Settings section for a built-in's inputs: loads them, shows `children` with a `put` that saves some, or the
 * [`CustomizedNotice`] when the workflow is customized or off, and the Customize as a workflow link.
 */
export function BuiltinSettingsSection({ calls, queryKey, label, description, follows, off, children }: {
  calls: BuiltinSettingsApi;
  queryKey: readonly unknown[];
  label: string;
  description: string;
  follows: string;
  off: string;
  children: (v: BuiltinSettingsView, put: PutInputs) => ReactNode;
}) {
  const qc = useQueryClient();
  const view = useQuery({ queryKey, queryFn: calls.get });
  const done = (v: BuiltinSettingsView) => {
    qc.setQueryData(queryKey, v);
    invalidateWorkflow(qc, v.workflowId);
  };
  const save = useMutation({ meta: { errorShown: true }, mutationFn: calls.save, onSuccess: done });
  const reset = useMutation({ meta: { errorShown: true }, mutationFn: calls.reset, onSuccess: done });
  const v = view.data;
  return (
    <Section
      label={label}
      description={description}
      actions={v && <Link onClick={() => openWorkflow(v.workflowId)}>Customize as a workflow →</Link>}
    >
      {!v ? (
        view.error ? <Alert tone="error">Could not load it: {view.error.message}</Alert> : <Skeleton label="Loading" className="h-24" />
      ) : !v.enabled || v.customized ? (
        <CustomizedNotice name={v.name} workflowId={v.workflowId} follows={follows} off={v.enabled ? undefined : off}
          onReset={() => reset.mutate()} resetting={reset.isPending} />
      ) : (
        children(v, (values) => save.mutateAsync(values))
      )}
    </Section>
  );
}

/** A chat channel input (`id`): a connection of a kind a plugin that is on posts to, or none. */
export function ChatChannelRow({ v, id, put, what }: {
  v: { values: Record<string, unknown>; channels: NotificationChannel[] };
  id: string;
  put: (values: Record<string, unknown>) => Promise<unknown>;
  /** What is posted: "Post it", "Post the daily update". */
  what: string;
}) {
  const connections = useConnections();
  const kinds = new Set(v.channels.map((c) => c.connectionKind));
  const label = (kind: string) => v.channels.find((c) => c.connectionKind === kind)?.label ?? kind;
  const usable = (connections.data ?? []).filter((c) => kinds.has(c.kind));
  const chat = typeof v.values[id] === 'string' ? v.values[id] : '';
  const options = [
    { value: '', label: 'None' },
    ...usable.map((c) => ({ value: c.name, label: `${c.name} (${label(c.kind)})` })),
    ...(chat && !usable.some((c) => c.name === chat) ? [{ value: chat, label: `${chat} (missing)` }] : []),
  ];
  return (
    <AutoSaveRow label="Chat channel" description={`${what} to ${v.channels.map((c) => c.label).join(' or ')} as well. Add a connection in Settings → Plugins.`}>
      {(track) => (
        <Select<string> aria-label="Chat channel" className="w-56" options={options} value={chat}
          onChange={(name) => void track(put({ [id]: name }))} />
      )}
    </AutoSaveRow>
  );
}
