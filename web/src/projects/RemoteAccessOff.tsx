import { Alert, Button } from '@ui';
import { navigate } from '../shell/router';

const REMOTE_ACCESS_PATH = '/settings/host/remote-access';

/**
 * In place of pairing (Add instance, Join) while remote access is off: linked instances reach each other through it.
 * `onLeave` runs before it goes to the setting (a dialog closes).
 */
export function RemoteAccessOff({ onLeave }: { onLeave?: () => void }) {
  return (
    <Alert
      tone="warning"
      action={<Button size="sm" onClick={() => { onLeave?.(); navigate(REMOTE_ACCESS_PATH); }}>Turn on remote access →</Button>}
    >
      Remote access is off, so other instances can&apos;t reach this one.
    </Alert>
  );
}
