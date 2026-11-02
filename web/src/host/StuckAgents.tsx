import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Link, Section, Select, SettingsRow, Skeleton, Switch } from '@ui';
import { api } from '../api/client';
import type { StuckAgentsView } from '../api/gen/StuckAgentsView';
import { AutoSaveRow, AutoSaveText } from '../common/AutoSave';
import { CustomizedNotice, openWorkflow } from '../common/BuiltinSettings';
import { invalidateWorkflow } from '../workflows/invalidate';

const KEY = ['stuck-agents'];

const text = (v: unknown) => (typeof v === 'string' ? v : '');
const num = (v: unknown) => (typeof v === 'number' ? v : 0);

const nudgeChoices = [0, 1, 2, 3, 4, 5].map((n) => ({ value: String(n), label: n === 0 ? 'None' : String(n) }));
const restartChoices = [0, 1, 2, 3].map((n) => ({
  value: String(n),
  label: n === 0 ? 'Ask me' : n === 1 ? 'Restart once, then ask me' : `Restart ${n} times, then ask me`,
}));
const loopChoices = [{ value: 'ask', label: 'Ask me' }, { value: 'stop', label: 'Stop the step' }];

/**
 * Settings → Agents → When an agent is stuck (PDT-367): the built-in workflow's inputs. A workflow changed beyond them
 * in the editor is only named here, with Open and Reset to default.
 */
export function StuckAgents() {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: KEY, queryFn: api.stuckAgents });
  const done = (v: StuckAgentsView) => {
    qc.setQueryData(KEY, v);
    void qc.invalidateQueries({ queryKey: ['workflow', v.workflowId] });
  };
  const save = useMutation({ meta: { errorShown: true }, mutationFn: api.saveStuckAgents, onSuccess: done });
  const reset = useMutation({ mutationFn: api.resetStuckAgents, onSuccess: done });
  const enable = useMutation({
    meta: { errorShown: true },
    mutationFn: ({ id, on }: { id: string; on: boolean }) => api.patchWorkflow(id, { enabled: on }),
    onSuccess: (_d, { id }) => invalidateWorkflow(qc, id),
  });
  const v = view.data;
  const put = (values: Record<string, unknown>) => save.mutateAsync(values);
  return (
    <Section
      label="When an agent is stuck"
      description="What happens when an agent step stops without an outcome, reaches its loop limit or runs out of time."
      actions={v && <Link onClick={() => openWorkflow(v.workflowId)}>Customize as a workflow →</Link>}
    >
      {!v ? (
        view.error ? <Alert tone="error">Could not load it: {view.error.message}</Alert> : <Skeleton label="Loading" className="h-24" />
      ) : (
        <>
          <AutoSaveRow
            label="Handle stuck agents"
            description={v.enabled ? `Runs the workflow ${v.name}.` : 'Off: a stalled agent is nudged once, then you are asked in the Inbox.'}
          >
            {(track) => (
              <Switch aria-label="Handle stuck agents" checked={v.enabled} onChange={(on) => void track(enable.mutateAsync({ id: v.workflowId, on }))} />
            )}
          </AutoSaveRow>
          {v.customized ? (
            <CustomizedNotice name={v.name} workflowId={v.workflowId} follows="Stuck agents follow" onReset={() => reset.mutate()} resetting={reset.isPending} />
          ) : (
            <Inputs v={v} put={put} />
          )}
        </>
      )}
    </Section>
  );
}

function Inputs({ v, put }: { v: StuckAgentsView; put: (values: Record<string, unknown>) => Promise<StuckAgentsView> }) {
  const value = (id: string) => v.values[id];
  const message = (id: string, label: string, description: string) => (
    <SettingsRow label={label} description={description} stacked>
      <AutoSaveText multiline rows={3} aria-label={label} className="w-full" value={text(value(id))} onSave={(t) => put({ [id]: t })} />
      {text(value(id)) !== text(v.defaults[id]) && (
        <Link className="self-start text-xs" onClick={() => void put({ [id]: v.defaults[id] })}>Reset to default text</Link>
      )}
    </SettingsRow>
  );
  return (
    <>
      <AutoSaveRow label="Nudges" description="How many times a stalled agent is nudged before it is restarted or you are asked.">
        {(track) => (
          <Select<string> aria-label="Nudges" className="w-28" options={nudgeChoices} value={String(num(value('nudges')))}
            onChange={(n) => void track(put({ nudges: Number(n) }))} />
        )}
      </AutoSaveRow>
      {message('firstNudge', 'First nudge', 'What the agent is told the first time it stops without an outcome.')}
      {message('laterNudge', 'Later nudges', 'What it is told after that. {{trigger.goal}} is the step’s task, {{trigger.lastMessage}} the end of what the agent last said.')}
      <AutoSaveRow label="Restart in a fresh session before asking you" description="The step starts again in a new session with its task, the earlier session’s last message and the branch’s state.">
        {(track) => (
          <Switch aria-label="Restart in a fresh session before asking you" checked={value('restart') === true}
            onChange={(on) => void track(put({ restart: on }))} />
        )}
      </AutoSaveRow>
      <AutoSaveRow label="At a loop limit" description="When a step is about to run more times in one run than its loop limit allows.">
        {(track) => (
          <Select<string> aria-label="At a loop limit" className="w-40" options={loopChoices} value={text(value('onLoopLimit')) || 'ask'}
            onChange={(c) => void track(put({ onLoopLimit: c }))} />
        )}
      </AutoSaveRow>
      <AutoSaveRow label="At a time limit" description="When a step has no outcome within its time limit or its turns.">
        {(track) => (
          <Select<string> aria-label="At a time limit" className="w-64 max-w-full" options={restartChoices} value={String(num(value('timeoutRestarts')))}
            onChange={(n) => void track(put({ timeoutRestarts: Number(n) }))} />
        )}
      </AutoSaveRow>
    </>
  );
}
