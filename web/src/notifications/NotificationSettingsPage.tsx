import { useState } from 'react';
import { Alert, Button, Page, Section, Select, SettingsRow, Skeleton, Switch, toast } from '@ui';
import type { KindSetting } from '../api/gen/KindSetting';
import type { Show } from '../api/gen/Show';
import { actionError } from '../common/useAction';
import { desktopNotificationsOn, setDesktopNotifications, showDesktop } from './desktop';
import { useNotificationSettings, useSetNotificationSettings } from './queries';

export const SHOW_LABEL: Record<Show, string> = {
  off: 'Off',
  dot: 'Dot only',
  inbox: 'Inbox',
  toast: 'Inbox and toast',
  desktop: 'Desktop notification',
};

/**
 * Settings → Notifications (PDT-108 §7): this device's desktop notifications, and what each kind of notification
 * shows (a host setting: every device sees the same Inbox).
 */
export function NotificationSettingsPage() {
  const settings = useNotificationSettings();
  const set = useSetNotificationSettings();
  const kinds = settings.data;
  const change = (c: Record<string, Show | null>) => set.mutate(c, { onError: (err) => toast.error(actionError(err)) });
  const changed = (kinds ?? []).filter((k) => k.show !== k.defaultShow);
  return (
    <Page
      title="Notifications"
      description="Needs you counts in the Inbox's badge and the page title; Updates wait on nothing."
      actions={
        <Button size="sm" variant="secondary" disabled={changed.length === 0} pending={set.isPending}
          onClick={() => change(Object.fromEntries(changed.map((k) => [k.kind, null])))}>
          Reset to defaults
        </Button>
      }
    >
      <ThisDevice />
      {!kinds ? (
        settings.error ? (
          <Alert tone="error">Notification settings could not be loaded: {settings.error.message}</Alert>
        ) : (
          <Skeleton label="Loading notification settings" className="h-40" />
        )
      ) : (
        <>
          <Kinds title="Needs you" description="Waits on you. Questions, gates and other steps never go below the Inbox, so they stay findable."
            kinds={kinds.filter((k) => k.section === 'needs_you')} onChange={change} />
          <Kinds title="Updates" description="Worth knowing. Each level includes the ones before it."
            kinds={kinds.filter((k) => k.section === 'updates')} onChange={change} />
        </>
      )}
    </Page>
  );
}

function Kinds({ title, description, kinds, onChange }: {
  title: string;
  description: string;
  kinds: KindSetting[];
  onChange: (c: Record<string, Show | null>) => void;
}) {
  return (
    <Section label={title} description={description}>
      {kinds.map((k) => (
        <SettingsRow key={k.kind} label={k.label} description={k.show === k.defaultShow ? k.when : `${k.when} Default: ${SHOW_LABEL[k.defaultShow]}.`}>
          <Select<Show>
            aria-label={`${k.label}: show`}
            className="w-52"
            options={k.allowed.map((s) => ({ value: s, label: SHOW_LABEL[s] }))}
            value={k.show}
            onChange={(s) => onChange({ [k.kind]: s })}
          />
        </SettingsRow>
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
