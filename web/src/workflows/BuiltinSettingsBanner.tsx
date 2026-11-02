import { Banner, Button } from '@ui';
import { routePath, type SettingsPage } from '../shell/route';
import { navigate } from '../shell/router';

/** The built-ins whose settings live in Settings (PDT-367, 368, 369, 370, 384): where, and what is set there. One line each. */
const SETTINGS_HOMES: Record<string, { page: SettingsPage; where: string; what: string }> = {
  'stuck-agents': { page: { kind: 'host', section: 'agents' }, where: 'Settings → This host → Agents', what: 'its nudges, messages and limits' },
  'notification-rules': { page: { kind: 'notifications' }, where: 'Settings → Notifications', what: 'its levels, quiet hours, escalation and chat channel' },
  'install-updates': { page: { kind: 'host', section: 'updates' }, where: 'Settings → This host → Updates', what: 'the time window, waiting for agents and the check after installing' },
  'pm-daily-checkpoint': { page: { kind: 'host', section: 'pm' }, where: 'Settings → This host → PM', what: 'the time, the days and the chat channel' },
  'clean-up-storage': { page: { kind: 'host', section: 'storage' }, where: 'Settings → This host → Storage', what: 'the low and almost-full thresholds and what happens when a disk is almost full' },
};

/** On a built-in Settings sets: its settings are in Settings, and changing more here makes it the user's own. */
export function BuiltinSettingsBanner({ builtin }: { builtin: string | null }) {
  const home = builtin ? SETTINGS_HOMES[builtin] : undefined;
  if (!home) return null;
  return (
    <Banner tone="neutral" actions={<Button size="sm" variant="ghost" onClick={() => navigate(routePath({ view: 'settings', page: home.page }))}>Open Settings</Button>}>
      Its settings are in {home.where}: {home.what}. Changing more here makes it your own: Settings then only offers Reset to default.
    </Banner>
  );
}
