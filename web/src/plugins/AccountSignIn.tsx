// A plugin's account sign-in in the browser (PDT-316, PDT-378), from what it declares (`SignIn`): each account signs
// in on the service's consent page, in a window of its own, which comes back to PDT and closes. With PDT's own
// client (`builtIn`) Sign in with … adds an account, and each account lets agents in with a consent of its own; an
// account with the user's own OAuth client is under Advanced.
import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Section, Skeleton } from '@ui';
import { api } from '../api/client';
import type { SignIn } from '../api/gen/SignIn';
import type { SignInAccount } from '../api/gen/SignInAccount';
import type { AccountSignInStart } from '../api/gen/AccountSignInStart';
import { actionError } from '../common/useAction';
import { ConnectionDialog } from '../connections/ConnectionDialog';

export const signInAccountsKey = (url: string) => ['sign-in', 'accounts', url] as const;

/** The accounts at `url` and their sign-in state; read again quickly while a sign-in is under way. */
export function useSignInAccounts(url: string | null, waiting = false) {
  return useQuery({
    queryKey: signInAccountsKey(url ?? ''),
    queryFn: () => api.signInAccounts(url!),
    refetchInterval: waiting ? 2000 : false,
    enabled: url !== null,
  });
}

/** The address the service sends the browser back to: it must be one of the OAuth client's redirect URIs. */
export const redirectUri = (s: SignIn) => `${window.location.origin}${s.api}/callback`;

/** Each account's sign-in, on its plugin's page. */
export function AccountSignIn({ signIn: s }: { signIn: SignIn }) {
  const qc = useQueryClient();
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ownClient, setOwnClient] = useState(false);
  const url = `${s.api}/accounts`;
  const accounts = useSignInAccounts(url, waiting);
  const builtIn = s.builtIn;
  const refresh = () => void qc.invalidateQueries({ queryKey: signInAccountsKey(url) });
  // The callback page tells this one when it is done.
  useEffect(() => {
    const done = (e: MessageEvent) => {
      if (e.origin !== window.location.origin || typeof e.data !== 'object' || e.data === null || !('pdtSignIn' in e.data)) return;
      setWaiting(false);
      void qc.invalidateQueries({ queryKey: signInAccountsKey(url) });
    };
    window.addEventListener('message', done);
    return () => window.removeEventListener('message', done);
  }, [qc, url]);
  /** Opens the service's consent page for `start` in a window of its own. */
  const consent = async (start: () => Promise<AccountSignInStart>) => {
    setError(null);
    // Opened now, in the click, so the browser doesn't block it; it goes to the service once PDT says where.
    const w = window.open('', 'pdt-sign-in', 'popup,width=520,height=680');
    try {
      const { url: to } = await start();
      if (w) w.location.href = to;
      else window.location.assign(to);
      setWaiting(true);
    } catch (e) {
      w?.close();
      setError(actionError(e));
    }
  };
  const act = async (f: () => Promise<void>) => {
    setError(null);
    try {
      await f();
      refresh();
    } catch (e) {
      setError(actionError(e));
    }
  };
  const origin = window.location.origin;
  const redirectNote = <>Add <code className="text-foreground">{redirectUri(s)}</code> to your OAuth client’s authorized redirect URIs first.</>;
  return (
    <Section
      label="Sign in"
      description={builtIn ? s.description : <>Each account signs in with {s.service}. {redirectNote}</>}
      actions={builtIn && <Button size="sm" variant="primary" onClick={() => void consent(() => api.signInAdd(s.api, origin))}>Sign in with {s.service}</Button>}
    >
      {accounts.isPending ? <Skeleton label={`Loading ${s.service} accounts`} className="h-8" />
        : accounts.isError ? <Alert tone="error">{s.service} accounts could not be loaded: {accounts.error.message}</Alert>
          : accounts.data.length === 0 ? <p className="text-sm text-muted-foreground">{builtIn ? `No ${s.service} account yet.` : `Add a connection for your ${s.service} account first.`}</p>
            : accounts.data.map((a: SignInAccount) => (
              <div key={a.connectionId} className="flex flex-wrap items-center gap-2 py-1">
                <span className="min-w-0 truncate text-sm font-medium">{a.name}</span>
                {a.signedIn
                  ? <span className="min-w-0 truncate text-xs text-muted-foreground">Signed in{a.email && a.email !== a.name ? ` as ${a.email}` : ''}</span>
                  : <Badge tone="waiting">Not signed in</Badge>}
                {!a.builtIn && <span className="text-xs text-muted-foreground">Own OAuth client</span>}
                {a.signedIn && a.builtIn && a.agents && <Badge>Agents allowed</Badge>}
                {a.note && <span className="text-xs text-muted-foreground">{a.note}</span>}
                <span className="flex-1" />
                {a.signedIn && a.builtIn && (a.agents
                  ? <Button size="sm" variant="secondary" onClick={() => void act(() => api.forgetAgents(s.api, a.connectionId))}>Stop agents</Button>
                  : <Button size="sm" variant="secondary" onClick={() => void consent(() => api.signInAccount(s.api, a.connectionId, origin, true))}>Allow agents</Button>)}
                {a.signedIn && <Button size="sm" variant="secondary" onClick={() => void act(() => api.signOutAccount(s.api, a.connectionId))}>Sign out</Button>}
                <Button size="sm" variant={a.signedIn ? 'secondary' : 'primary'} disabled={!a.hasClient} onClick={() => void consent(() => api.signInAccount(s.api, a.connectionId, origin))}>
                  {a.signedIn ? 'Sign in again' : 'Sign in'}
                </Button>
              </div>
            ))}
      {waiting && <p role="status" className="text-xs text-muted-foreground">Finish signing in in the {s.service} window.</p>}
      {error && <Alert tone="error">{error}</Alert>}
      {builtIn && (
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">Advanced</summary>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <span className="min-w-0 flex-1">{s.ownClient} {redirectNote}</span>
            <Button size="sm" variant="secondary" onClick={() => setOwnClient(true)}>Use your own OAuth client</Button>
          </div>
        </details>
      )}
      {ownClient && <ConnectionDialog kind={s.kind} onClose={() => { setOwnClient(false); refresh(); }} />}
    </Section>
  );
}
