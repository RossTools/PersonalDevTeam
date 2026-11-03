import { useState } from 'react';
import { Button, Link, Page, Section, Select, SettingsRow, Switch, toast } from '@ui';
import type { KindSetting } from '../api/gen/KindSetting';
import type { NotificationRulesInput } from '../api/gen/NotificationRulesInput';
import type { NotificationRulesView } from '../api/gen/NotificationRulesView';
import type { Show } from '../api/gen/Show';
import { AutoSaveRow, AutoSaveText } from '../common/AutoSave';
import { CustomizedNotice, openWorkflow } from '../common/BuiltinSettings';
import { useConnections } from '../connections/queries';
import { desktopNotificationsOn, setDesktopNotifications, showDesktop } from './desktop';
import { useNotificationRules, useNotificationRulesActions } from './queries';
import { QueryStatus } from '../common/QueryStatus';

export const SHOW_LABEL: Record<Show, string> = {
  off: 'Off',
  dot: 'Dot only',
  inbox: 'Inbox',
  toast: 'Inbox and toast',
  desktop: 'Desktop notification',
};

const HOUR = 60 * 60 * 1000;
const escalateChoices = [0.5, 1, 2, 4, 8, 24].map((h) => ({
  value: String(h * HOUR),
  label: h < 1 ? '30 minutes' : h === 1 ? '1 hour' : `${h} hours`,
}));

const text = (v: unknown) => (typeof v === 'string' ? v : '');

type Put = (input: NotificationRulesInput) => Promise<NotificationRulesView>;

/**
 * Settings → Notifications (PDT-108, PDT-368): this device's desktop notifications, and the Notification rules
 * workflow's inputs: what each kind of notification shows (every device sees the same Inbox), escalation, quiet hours
 * and a chat channel. A workflow changed beyond its inputs is only named here, with Open and Reset to default.
 */
export function NotificationSettingsPage() {
  const rules = useNotificationRules();
  const { save, reset, enable } = useNotificationRulesActions();
  const v = rules.data;
  const put: Put = (input) => save.mutateAsync(input);
  const changed = (v?.kinds ?? []).filter((k) => k.show !== k.defaultShow);
  const editable = !!v && !v.customized;
  return (
    <Page
      title="Notifications"
      description="Needs you counts in the Inbox's badge and the page title; Updates wait on nothing."
      actions={
        <Button size="sm" variant="secondary" disabled={!editable || changed.length === 0} pending={save.isPending}
          onClick={() => void put({ levels: Object.fromEntries(changed.map((k) => [k.kind, null])) })}>
          Reset to defaults
        </Button>
      }
    >
      <ThisDevice />
      {!v ? (
        <QueryStatus query={rules} what="Notification settings" skeleton="h-40">{() => null}</QueryStatus>
      ) : (
        <>
          <Section
            label="Rules"
            description="Notifications follow the Notification rules workflow; these are its settings."
            actions={<Link onClick={() => openWorkflow(v.workflowId)}>Customize as a workflow →</Link>}
          >
            <AutoSaveRow
              label="Notification rules"
              description={v.enabled
                ? `Runs the workflow ${v.name}.`
                : 'Off: every kind shows at PDT’s default level, without quiet hours, escalation or a chat channel.'}
            >
              {(track) => (
                <Switch aria-label="Notification rules" checked={v.enabled}
                  onChange={(on) => void track(enable.mutateAsync({ id: v.workflowId, on }))} />
              )}
            </AutoSaveRow>
            {v.customized ? (
              <CustomizedNotice name={v.name} workflowId={v.workflowId} follows="Notifications follow" onReset={() => reset.mutate()} resetting={reset.isPending} />
            ) : (
              <Rules v={v} put={put} />
            )}
          </Section>
          <Kinds title="Needs you" description="Waits on you. Questions, gates and other steps never go below the Inbox, so they stay findable."
            kinds={v.kinds.filter((k) => k.section === 'needs_you')} editable={editable} put={put} />
          <Kinds title="Updates" description="Worth knowing. Each level includes the ones before it."
            kinds={v.kinds.filter((k) => k.section === 'updates')} editable={editable} put={put} />
        </>
      )}
    </Page>
  );
}

/** Escalation, quiet hours and the chat channel. */
function Rules({ v, put }: { v: NotificationRulesView; put: Put }) {
  const value = (id: string) => v.values[id];
  const set = (id: string, x: unknown) => put({ values: { [id]: x } });
  const flag = (id: string, label: string, description: string) => (
    <AutoSaveRow label={label} description={description}>
      {(track) => <Switch aria-label={label} checked={value(id) === true} onChange={(on) => void track(set(id, on))} />}
    </AutoSaveRow>
  );
  const after = String(value('escalateAfter'));
  return (
    <>
      {flag('escalate', 'Escalate what waits', 'An Inbox item still unanswered after a while comes back as a desktop notification, and to the chat channel if one is set.')}
      {value('escalate') === true && (
        <AutoSaveRow label="Escalate after" description="How long an Inbox item waits unanswered first.">
          {(track) => (
            <Select<string> aria-label="Escalate after" className="w-40"
              options={escalateChoices.some((c) => c.value === after) ? escalateChoices : [...escalateChoices, { value: after, label: `${Math.round(Number(after) / 60000)} minutes` }]}
              value={after} onChange={(ms) => void track(set('escalateAfter', Number(ms)))} />
          )}
        </AutoSaveRow>
      )}
      {flag('quietHours', 'Quiet hours', 'No toasts or desktop notifications between two times. They wait in the Inbox, and one summary comes when quiet hours end.')}
      {value('quietHours') === true && (
        <SettingsRow label="Quiet from" description="From and until, as HH:MM in this computer’s time.">
          <AutoSaveText aria-label="Quiet from" className="w-24" value={text(value('quietFrom'))} onSave={(t) => set('quietFrom', t.trim())} />
          <span className="text-sm text-muted-foreground">until</span>
          <AutoSaveText aria-label="Quiet until" className="w-24" value={text(value('quietTo'))} onSave={(t) => set('quietTo', t.trim())} />
        </SettingsRow>
      )}
      {v.channels.length > 0 && <Chat v={v} put={put} />}
    </>
  );
}

/** The chat channel questions, approvals and failed runs post to: a connection of a kind a plugin posts to. */
function Chat({ v, put }: { v: NotificationRulesView; put: Put }) {
  const connections = useConnections();
  const kinds = new Set(v.channels.map((c) => c.connectionKind));
  const label = (kind: string) => v.channels.find((c) => c.connectionKind === kind)?.label ?? kind;
  const usable = (connections.data ?? []).filter((c) => kinds.has(c.kind));
  const chat = text(v.values.chat);
  const options = [
    { value: '', label: 'None' },
    ...usable.map((c) => ({ value: c.name, label: `${c.name} (${label(c.kind)})` })),
    ...(chat && !usable.some((c) => c.name === chat) ? [{ value: chat, label: `${chat} (missing)` }] : []),
  ];
  const services = v.channels.map((c) => c.label).join(' or ');
  const set = (id: string, x: unknown) => put({ values: { [id]: x } });
  return (
    <>
      <AutoSaveRow label="Chat channel" description={`Post what waits on you and what fails to ${services}. Add a connection in Settings → Plugins.`}>
        {(track) => (
          <Select<string> aria-label="Chat channel" className="w-56" options={options} value={chat}
            onChange={(name) => void track(set('chat', name))} />
        )}
      </AutoSaveRow>
      {chat && ([
        ['chatQuestions', 'Post questions', 'An agent asks you something.'],
        ['chatApprovals', 'Post approvals', 'A conversation asks to use a tool, or asks you to confirm.'],
        ['chatFailedRuns', 'Post failed runs', 'A workflow run fails.'],
      ] as const).map(([id, l, d]) => (
        <AutoSaveRow key={id} label={l} description={d}>
          {(track) => <Switch aria-label={l} checked={v.values[id] === true} onChange={(on) => void track(set(id, on))} />}
        </AutoSaveRow>
      ))}
    </>
  );
}

function Kinds({ title, description, kinds, editable, put }: {
  title: string;
  description: string;
  kinds: KindSetting[];
  editable: boolean;
  put: Put;
}) {
  return (
    <Section label={title} description={description}>
      {kinds.map((k) => (
        <AutoSaveRow key={k.kind} label={k.label} description={k.show === k.defaultShow ? k.when : `${k.when} Default: ${SHOW_LABEL[k.defaultShow]}.`}>
          {(track) => (
            <Select<Show>
              aria-label={`${k.label}: show`}
              className="w-52"
              disabled={!editable}
              options={k.allowed.map((s) => ({ value: s, label: SHOW_LABEL[s] }))}
              value={k.show}
              onChange={(s) => void track(put({ levels: { [k.kind]: s } }))}
            />
          )}
        </AutoSaveRow>
      ))}
    </Section>
  );
}

/** Per browser: desktop notifications while the page is hidden. */
function ThisDevice() {
  const supported = typeof Notification !== 'undefined';
  const [on, setOn] = useState(desktopNotificationsOn);
  const [denied, setDenied] = useState(false);
  const toggle = (want: boolean) => {
    void setDesktopNotifications(want).then((now) => {
      setOn(now);
      setDenied(want && !now);
    });
  };
  const test = () => {
    if (!showDesktop('Test notification', 'Desktop notifications work on this device.', 'test', () => {})) {
      toast.error('This browser would not show it.');
    }
  };
  return (
    <Section label="This device">
      <SettingsRow
        label="Desktop notifications"
        description={supported
          ? 'Kinds set to Desktop notification show on this device\'s desktop while the page is hidden. This browser only; one tab shows them.'
          : 'This browser cannot show desktop notifications.'}
      >
        {denied && <span className="text-xs text-destructive">The browser blocked them: allow notifications for this site.</span>}
        <Button size="sm" variant="secondary" disabled={!on} onClick={test}>Test</Button>
        <Switch aria-label="Desktop notifications" checked={on} disabled={!supported} onChange={toggle} />
      </SettingsRow>
    </Section>
  );
}
