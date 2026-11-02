import { useQuery } from '@tanstack/react-query';
import { useState, type ReactNode } from 'react';
import { Alert, Button } from '@ui';
import { api } from '../api/client';
import { useAction } from '../common/useAction';
import { linkExpired, PAIRING_KEY } from './queries';

/**
 * The app, once this browser is paired (PDT-401): only a paired browser gets the UI token, so an agent's curl can't
 * make the changes only the user may make. Unpaired, the pairing screen instead. While it is unknown (loading, an
 * older pdtd without pairing) the app shows.
 */
export function PairGate({ children }: { children: ReactNode }) {
  const pairing = useQuery({
    queryKey: PAIRING_KEY,
    queryFn: api.pairing,
    retry: false,
    // Pairing in another tab of this browser pairs this one too: the cookie is shared.
    refetchInterval: (q) => (q.state.data?.paired === false ? 2000 : false),
  });
  if (pairing.data?.paired === false) return <PairScreen />;
  return <>{children}</>;
}

export function PairScreen() {
  const whoami = useQuery({ queryKey: ['host-whoami'], queryFn: api.whoami, retry: false });
  const [opened, setOpened] = useState(false);
  const [open, opening] = useAction(async () => {
    await api.openPaired();
    setOpened(true);
  });
  const local = whoami.data?.caller !== 'tailnet';
  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6 text-sm">
      <div className="flex max-w-md flex-col gap-4">
        <h1 className="text-base font-semibold">Pair this browser</h1>
        <p className="text-muted-foreground">
          PDT takes approvals and the other changes only you may make from a browser you&apos;ve paired, so an agent on
          this computer can&apos;t make them for you.
        </p>
        {linkExpired && <Alert tone="warning">That pairing link was already used or is too old. Get a new one.</Alert>}
        {local && (
          <div className="flex flex-col gap-2">
            <div>
              <Button pending={opening} onClick={() => void open()}>Pair this browser</Button>
            </div>
            {opened ? (
              <p className="text-muted-foreground">
                PDT opened itself, paired, in this computer&apos;s default browser. If this tab doesn&apos;t carry on by
                itself, use that one.
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">PDT opens itself in this computer&apos;s default browser, paired.</p>
            )}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          On another device or browser: in a browser you&apos;ve paired, open Settings › This host › Browsers › Pair
          another browser, and open its link here.
        </p>
      </div>
    </main>
  );
}
