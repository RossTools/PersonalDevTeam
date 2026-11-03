// Settings › Startup (PDT-489): whether PDT's background service starts at login. The tray icon has its own
// "Start at login"; this one is the service that runs PDT.
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Section, Skeleton, Switch } from '@ui';
import { api } from '../api/client';
import { AutoSaveRow } from '../common/AutoSave';

export const START_AT_LOGIN_KEY = ['host-start-at-login'];

export function StartAtLogin() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: START_AT_LOGIN_KEY, queryFn: api.startAtLogin });
  const v = q.data;
  return (
    <Section label="Startup" description="PDT runs as a background service on this computer, apart from its tray icon.">
      {!v ? (
        q.error ? <Alert tone="error">Could not read whether PDT starts at login: {q.error.message}</Alert>
          : <Skeleton label="Loading" className="h-10" />
      ) : (
        <AutoSaveRow
          label="Start PDT at login"
          description={v.unavailable ? `Can't be changed here: ${v.unavailable}.`
            : 'Starts PDT when you log in. Turned off, PDT keeps running until you stop it or log out.'}
        >
          {(track) => (
            <Switch
              aria-label="Start PDT at login"
              checked={v.on}
              disabled={v.unavailable !== null}
              onChange={(on) => void track(api.setStartAtLogin(on).then((next) => qc.setQueryData(START_AT_LOGIN_KEY, next)))}
            />
          )}
        </AutoSaveRow>
      )}
    </Section>
  );
}
