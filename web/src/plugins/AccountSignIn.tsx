// A plugin's account sign-in in the browser (PDT-316, PDT-378), from what it declares (`SignIn`): each account signs
// in on the service's consent page, in a window of its own, which comes back to PDT and closes. With PDT's own
// client (`builtIn`) the window starts at the website's page (`relay`), which goes back only to the PDT that hands it
// the consent page (PDT-410); when that page can't hand the answer back (a corporate proxy), it shows a code to paste
// here instead (PDT-590). Sign in with … adds an account, and each account lets agents in with a consent of its own; an
// account with the user's own OAuth client is under Advanced.
import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Disclosure, Input, Section, Skeleton } from '@ui';
import { api } from '../api/client';
import type { SignIn } from '../api/gen/SignIn';
import type { SignInAccount } from '../api/gen/SignInAccount';
import type { AccountSignInStart } from '../api/gen/AccountSignInStart';
import { actionError } from '../common/useAction';
import { ConnectionDialog } from '../connections/ConnectionDialog';
import { connectionsKey } from '../connections/queries';
import { POLL } from '../common/poll';
import { LoadError } from '../common/QueryStatus';

// Under the connections' key, so saving or deleting a connection anywhere reads the accounts again (PDT-464).
const signInAccountsKey = (url: string) => [...connectionsKey, 'sign-in', url] as const;

/** The accounts at `url` and their sign-in state; read again quickly while a sign-in is under way. */
export function useSignInAccounts(url: string | null, waiting = false) {
  return useQuery({
    queryKey: signInAccountsKey(url ?? ''),
    queryFn: () => api.signInAccounts(url!),
    refetchInterval: waiting ? POLL.live : false,
    enabled: url !== null,
  });
}

/** The address the service sends the browser back to: it must be one of the OAuth client's redirect URIs. */
export const redirectUri = (s: SignIn) => `${window.location.origin}${s.api}/callback`;

/** How long the website's page has to say it is listening before the window goes to the consent page itself. */
export const RELAY_READY_MS = 8_000;
/** How long a sign-in through the website's page may take before PDT says what may be in the way. */
export const SLOW_SIGN_IN_MS = 90_000;

/** Each account's sign-in, on its plugin's page. */
export function AccountSignIn({ signIn: s }: { signIn: SignIn }) {
  const qc = useQueryClient();
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ownClient, setOwnClient] = useState(false);
  // The sign-in under way went through the website's page, so its answer may come back as a code to paste.
  const [relayed, setRelayed] = useState(false);
  const [slow, setSlow] = useState(false);
  const [pasting, setPasting] = useState(false);
  const [code, setCode] = useState('');
  const [done, setDone] = useState<string | null>(null);
  const url = `${s.api}/accounts`;
  const accounts = useSignInAccounts(url, waiting);
  const builtIn = s.builtIn;
  const refresh = () => void qc.invalidateQueries({ queryKey: signInAccountsKey(url) });
  // The relay page's handshake for the sign-in under way: a new one replaces it (the window is the same one).
  const handshake = useRef<((e: MessageEvent) => void) | null>(null);
  const slowTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const stopHandshake = () => {
    if (handshake.current) window.removeEventListener('message', handshake.current);
    handshake.current = null;
  };
  useEffect(() => () => {
    stopHandshake();
    clearTimeout(slowTimer.current);
  }, []);
  const finished = () => {
    stopHandshake();
    clearTimeout(slowTimer.current);
    setWaiting(false);
    setSlow(false);
    setPasting(false);
    setCode('');
  };
  // The callback page tells this one when it is done.
  useEffect(() => {
    const done = (e: MessageEvent) => {
      if (e.origin !== window.location.origin || typeof e.data !== 'object' || e.data === null || !('pdtSignIn' in e.data)) return;
      finished();
      void qc.invalidateQueries({ queryKey: signInAccountsKey(url) });
    };
    window.addEventListener('message', done);
    return () => window.removeEventListener('message', done);
  }, [qc, url]);
  /** Opens the service's consent page for `start` in a window of its own. */
  const consent = async (start: () => Promise<AccountSignInStart>) => {
    setError(null);
    setDone(null);
    finished();
    // Opened now, in the click, so the browser doesn't block it; it goes to the service once PDT says where.
    stopHandshake();
    const w = window.open('', 'pdt-sign-in', 'popup,width=520,height=680');
    try {
      const { url: to, relay } = await start();
      if (relay) {
        if (!w) throw new Error(`Allow pop-ups for PDT, then sign in again: ${s.service}’s sign-in needs a window of its own.`);
        // Once the relay page says it is listening, hand it the consent page; the browser tells it this PDT sent it.
        const from = new URL(relay).origin;
        const ready = (e: MessageEvent) => {
          if (e.source !== w || e.origin !== from || (e.data as { pdtRelay?: unknown } | null)?.pdtRelay !== 'ready') return;
          stopHandshake();
          w.postMessage({ pdtRelayTo: to }, from);
        };
        handshake.current = ready;
        window.addEventListener('message', ready);
        w.location.href = relay;
        // A page that can't reach this one (a proxy isolating it) never says it is ready: the window goes to the
        // consent page itself, and the answer comes back as a code to paste, never sent on. Whether the window was
        // closed can't be told (the consent page cuts it off from this one), so a slow sign-in says what may be wrong.
        setTimeout(() => {
          if (handshake.current !== ready) return;
          stopHandshake();
          w.location.href = to;
        }, RELAY_READY_MS);
        slowTimer.current = setTimeout(() => setSlow(true), SLOW_SIGN_IN_MS);
      } else if (w) w.location.href = to;
      else window.location.assign(to);
      setRelayed(!!relay);
      setWaiting(true);
    } catch (e) {
      w?.close();
      setError(actionError(e));
    }
  };
  const paste = async () => {
    setError(null);
    try {
      const { message } = await api.signInPaste(s.api, code.trim());
      finished();
      setDone(message);
      refresh();
    } catch (e) {
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
  const canPaste = relayed && s.pasteCode;
  const redirectNote = <>Add <code className="text-foreground">{redirectUri(s)}</code> to your OAuth client’s authorized redirect URIs first.</>;
  return (
    <Section
      label="Sign in"
      description={builtIn ? s.description : <>Each account signs in with {s.service}. {redirectNote}</>}
      actions={builtIn && <Button size="sm" variant="primary" onClick={() => void consent(() => api.signInAdd(s.api, origin))}>Sign in with {s.service}</Button>}
    >
      {accounts.isPending ? <Skeleton label={`Loading ${s.service} accounts`} className="h-8" />
        : accounts.isError ? <LoadError what={`${s.service} accounts`} error={accounts.error} />
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
      {waiting && (
        <div className="flex flex-wrap items-center gap-2">
          <p role="status" className="text-xs text-muted-foreground">Finish signing in in the {s.service} window.</p>
          {canPaste && !pasting && <Button size="sm" variant="ghost" onClick={() => setPasting(true)}>Paste a code instead</Button>}
          <Button size="sm" variant="ghost" onClick={finished}>Cancel</Button>
        </div>
      )}
      {waiting && slow && (
        <Alert tone="warning">
          Still not signed in? A company network may be blocking or isolating the sign-in page (a proxy or browser
          isolation), or your {s.service} Workspace admin may block apps they haven’t approved ({s.service} then says
          “Access blocked”, and only the admin can allow PDT).{canPaste && ' If the window shows a code, paste it here.'}
        </Alert>
      )}
      {waiting && pasting && (
        <form className="flex flex-wrap items-center gap-2" onSubmit={(e) => { e.preventDefault(); void paste(); }}>
          <Input mono className="min-w-0 flex-1" aria-label="Sign-in code" placeholder="The code the sign-in window shows" value={code} onChange={(e) => setCode(e.target.value)} autoFocus />
          <Button size="sm" variant="primary" type="submit" disabled={!code.trim()}>Finish signing in</Button>
        </form>
      )}
      {done && <Alert>{done}</Alert>}
      {error && <Alert tone="error">{error}</Alert>}
      {builtIn && (
        <Disclosure title="Advanced" panelClassName="mt-2 flex flex-wrap items-center gap-2">
          <span className="min-w-0 flex-1">{s.ownClient} {redirectNote}</span>
          <Button size="sm" variant="secondary" onClick={() => setOwnClient(true)}>Use your own OAuth client</Button>
        </Disclosure>
      )}
      {ownClient && <ConnectionDialog kind={s.kind} onClose={() => { setOwnClient(false); refresh(); }} />}
    </Section>
  );
}
