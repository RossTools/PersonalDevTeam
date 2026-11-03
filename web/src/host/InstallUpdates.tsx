import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Link, Section, Select, SettingsRow, Skeleton, Switch } from '@ui';
import { api } from '../api/client';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import type { UpdatePolicy } from '../api/gen/UpdatePolicy';
import { AutoSaveRow, AutoSaveText } from '../common/AutoSave';
import { ChatChannelRow, CustomizedNotice, openWorkflow, type PutInputs } from '../common/BuiltinSettings';
import { invalidateWorkflow } from '../workflows/invalidate';
import { plural } from '../common/text';

const KEY = ['install-updates'];

const text = (v: unknown) => (typeof v === 'string' ? v : '');

const HOUR = 60 * 60 * 1000;
const anywayChoices = [
  { value: '0', label: 'Wait for them to finish' },
  ...[1, 2, 4, 8, 24].map((h) => ({ value: String(h * HOUR), label: `Install anyway after ${plural(h, 'hour')}` })),
];

/**
 * Settings → Updates → Installing updates (PDT-369): the built-in Install updates workflow's inputs. A workflow
 * changed beyond them in the editor, or turned off, is only named here, with Open and Reset to default (or Turn on).
 */
export function InstallUpdates({ policy }: { policy: UpdatePolicy }) {
  const qc = useQueryClient();
  const view = useQuery({ queryKey: KEY, queryFn: api.installUpdates.get });
  const done = (v: BuiltinSettingsView) => {
    qc.setQueryData(KEY, v);
    void qc.invalidateQueries({ queryKey: ['workflow', v.workflowId] });
  };
  const save = useMutation({ meta: { errorShown: true }, mutationFn: api.installUpdates.save, onSuccess: done });
  const reset = useMutation({ meta: { errorShown: true }, mutationFn: api.installUpdates.reset, onSuccess: done });
  const enable = useMutation({
    meta: { errorShown: true },
    mutationFn: (id: string) => api.patchWorkflow(id, { enabled: true }),
    onSuccess: (_d, id) => invalidateWorkflow(qc, id),
  });
  const v = view.data;
  const auto = policy === 'auto_when_idle';
  return (
    <Section
      label="Installing updates"
      description={auto
        ? 'How updates install by themselves, and what happens after.'
        : 'What happens after an update installs. The rest applies once Install updates is set to install automatically when idle.'}
    >
      {!v ? (
        view.error ? <Alert tone="error">Could not load it: {view.error.message}</Alert> : <Skeleton label="Loading" className="h-24" />
      ) : !v.enabled ? (
        <Alert
          tone="info"
          action={
            <span className="flex gap-1">
              <Button size="sm" variant="secondary" onClick={() => openWorkflow(v.workflowId)}>Open</Button>
              <Button size="sm" variant="secondary" pending={enable.isPending} onClick={() => enable.mutate(v.workflowId)}>Turn on</Button>
            </span>
          }
        >
          The workflow {v.name} is off: updates install only when you say.
        </Alert>
      ) : v.customized ? (
        <CustomizedNotice name={v.name} workflowId={v.workflowId} follows="Updates follow" onReset={() => reset.mutate()} resetting={reset.isPending} />
      ) : (
        <Inputs v={v} put={(values) => save.mutateAsync(values)} auto={auto} />
      )}
      {v && <Link className="self-start text-sm" onClick={() => openWorkflow(v.workflowId)}>Customize as a workflow →</Link>}
    </Section>
  );
}

function Inputs({ v, put, auto }: { v: BuiltinSettingsView; put: PutInputs; auto: boolean }) {
  const value = (id: string) => v.values[id];
  const flag = (id: string, label: string, description: string, disabled = false) => (
    <AutoSaveRow label={label} description={description}>
      {(track) => (
        <Switch aria-label={label} checked={value(id) === true} disabled={disabled} onChange={(on) => void track(put({ [id]: on }))} />
      )}
    </AutoSaveRow>
  );
  const anyway = String(value('installAnywayAfter') ?? 0);
  const anywayOptions = anywayChoices.some((c) => c.value === anyway)
    ? anywayChoices
    : [...anywayChoices, { value: anyway, label: `Install anyway after ${Math.round(Number(anyway) / 60000)} minutes` }];
  return (
    <>
      {flag('window', 'Only in a time window', 'Updates install by themselves only between two times, such as overnight.', !auto)}
      {value('window') === true && (
        <SettingsRow label="Window from" description="From and until, as HH:MM in this computer’s time.">
          <AutoSaveText aria-label="Window from" className="w-24" value={text(value('windowFrom'))} disabled={!auto} onSave={(t) => put({ windowFrom: t })} />
          <span className="text-sm text-muted-foreground">until</span>
          <AutoSaveText aria-label="Window until" className="w-24" value={text(value('windowTo'))} disabled={!auto} onSave={(t) => put({ windowTo: t })} />
        </SettingsRow>
      )}
      <AutoSaveRow label="While agents work" description="An update waits until no agent is working or waiting on background tasks. Installing anyway stops them; sessions resume afterwards.">
        {(track) => (
          <Select<string> aria-label="While agents work" className="w-64 max-w-full" options={anywayOptions} value={anyway} disabled={!auto}
            onChange={(ms) => void track(put({ installAnywayAfter: Number(ms) }))} />
        )}
      </AutoSaveRow>
      {flag('stableOnly', 'Stable releases only', 'On the nightly channel, a nightly build doesn’t install by itself: install it from the banner.', !auto)}
      {flag('checkAfter', 'Check PDT after installing', 'A few minutes after PDT starts on the new version, check it runs that version and its background work keeps going. If not, it goes to your Inbox.')}
      {flag('announce', 'Tell me what changed', 'A notification linking to the release notes, once PDT runs the new version.')}
      {value('announce') === true && v.channels.length > 0 && <ChatChannelRow v={v} id="chat" put={put} what="Post it" />}
    </>
  );
}
