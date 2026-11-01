// The Google plugin in the browser (PDT-316): signing each Google account in (Google's consent page in a window of
// its own, which comes back to PDT and closes), and Google's file picker, which grants PDT the files the user picks.
// With PDT's own sign-in (PDT-360) Sign in with Google adds an account, and each account lets agents in with a consent
// of its own; an account with the user's own OAuth client is under Advanced.
import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Section, Skeleton } from '@ui';
import { api } from '../api/client';
import type { GoogleAccount } from '../api/gen/GoogleAccount';
import type { GoogleSignInStart } from '../api/gen/GoogleSignInStart';
import { actionError } from '../common/useAction';
import { ConnectionDialog } from '../connections/ConnectionDialog';

export const googleAccountsKey = ['google', 'accounts'] as const;
export const googleClientKey = ['google', 'client'] as const;

/** The Google accounts and their sign-in state; read again quickly while a sign-in is under way. */
export function useGoogleAccounts(waiting = false, enabled = true) {
  return useQuery({ queryKey: googleAccountsKey, queryFn: api.googleAccounts, refetchInterval: waiting ? 2000 : false, enabled });
}

/** Whether this PDT has its own Google sign-in. */
export function useGoogleClient(enabled = true) {
  return useQuery({ queryKey: googleClientKey, queryFn: api.googleClient, staleTime: Infinity, enabled });
}

/** The address Google sends the browser back to: it must be one of the OAuth client's redirect URIs. */
export const redirectUri = () => `${window.location.origin}/api/google/callback`;

/** Each Google account's sign-in, on the Google plugin's page. */
export function GoogleSignIn() {
  const qc = useQueryClient();
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [ownClient, setOwnClient] = useState(false);
  const accounts = useGoogleAccounts(waiting);
  const builtIn = useGoogleClient().data?.builtIn === true;
  // The callback page tells this one when it is done.
  useEffect(() => {
    const done = (e: MessageEvent) => {
      if (e.origin !== window.location.origin || typeof e.data !== 'object' || e.data === null || !('pdtGoogle' in e.data)) return;
      setWaiting(false);
      void qc.invalidateQueries({ queryKey: googleAccountsKey });
    };
    window.addEventListener('message', done);
    return () => window.removeEventListener('message', done);
  }, [qc]);
  const refresh = () => void qc.invalidateQueries({ queryKey: googleAccountsKey });
  /** Opens Google's consent page for `start` in a window of its own. */
  const consent = async (start: () => Promise<GoogleSignInStart>) => {
    setError(null);
    // Opened now, in the click, so the browser doesn't block it; it goes to Google once PDT says where.
    const w = window.open('', 'pdt-google-sign-in', 'popup,width=520,height=680');
    try {
      const { url } = await start();
      if (w) w.location.href = url;
      else window.location.assign(url);
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
  const redirectNote = <>Add <code className="text-foreground">{redirectUri()}</code> to your OAuth client’s authorized redirect URIs first.</>;
  return (
    <Section
      label="Sign in"
      description={builtIn
        ? 'Sign in with Google adds an account. PDT sees only the files it makes and those you pick, and agents use an account only once you allow them.'
        : <>Each account signs in with Google. {redirectNote}</>}
      actions={builtIn && <Button size="sm" variant="primary" onClick={() => void consent(() => api.googleAddAccount(origin))}>Sign in with Google</Button>}
    >
      {accounts.isPending ? <Skeleton label="Loading Google accounts" className="h-8" />
        : accounts.isError ? <Alert tone="error">Google accounts could not be loaded: {accounts.error.message}</Alert>
          : accounts.data.length === 0 ? <p className="text-sm text-muted-foreground">{builtIn ? 'No Google account yet.' : 'Add a connection for your Google account first.'}</p>
            : accounts.data.map((a: GoogleAccount) => (
              <div key={a.connectionId} className="flex flex-wrap items-center gap-2 py-1">
                <span className="min-w-0 truncate text-sm font-medium">{a.name}</span>
                {a.signedIn
                  ? <span className="min-w-0 truncate text-xs text-muted-foreground">Signed in{a.email && a.email !== a.name ? ` as ${a.email}` : ''}</span>
                  : <Badge tone="waiting">Not signed in</Badge>}
                {!a.builtIn && <span className="text-xs text-muted-foreground">Own OAuth client</span>}
                {a.signedIn && a.builtIn && a.agents && <Badge>Agents allowed</Badge>}
                {!a.picker && <span className="text-xs text-muted-foreground">No file picker (needs an API key and project number)</span>}
                <span className="flex-1" />
                {a.signedIn && a.builtIn && (a.agents
                  ? <Button size="sm" variant="secondary" onClick={() => void act(() => api.googleForgetAgents(a.connectionId))}>Stop agents</Button>
                  : <Button size="sm" variant="secondary" onClick={() => void consent(() => api.googleSignIn(a.connectionId, origin, true))}>Allow agents</Button>)}
                {a.signedIn && <Button size="sm" variant="secondary" onClick={() => void act(() => api.googleSignOut(a.connectionId))}>Sign out</Button>}
                <Button size="sm" variant={a.signedIn ? 'secondary' : 'primary'} disabled={!a.hasClient} onClick={() => void consent(() => api.googleSignIn(a.connectionId, origin))}>
                  {a.signedIn ? 'Sign in again' : 'Sign in'}
                </Button>
              </div>
            ))}
      {waiting && <p role="status" className="text-xs text-muted-foreground">Finish signing in in the Google window.</p>}
      {error && <Alert tone="error">{error}</Alert>}
      {builtIn && (
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">Advanced</summary>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <span className="min-w-0 flex-1">Your organization blocks apps from outside it? Add an account with your own OAuth client from Google Cloud. {redirectNote}</span>
            <Button size="sm" variant="secondary" onClick={() => setOwnClient(true)}>Use your own OAuth client</Button>
          </div>
        </details>
      )}
      {ownClient && <ConnectionDialog kind="google" onClose={() => { setOwnClient(false); refresh(); }} />}
    </Section>
  );
}

// Google's picker, as much of it as PDT uses (https://developers.google.com/drive/picker).
interface PickerDoc { id: string; name: string }
interface PickerData { action: string; docs?: PickerDoc[] }
interface PickerBuilder {
  addView(v: unknown): PickerBuilder;
  enableFeature(f: unknown): PickerBuilder;
  setOAuthToken(t: string): PickerBuilder;
  setDeveloperKey(k: string): PickerBuilder;
  setAppId(id: string): PickerBuilder;
  setTitle(t: string): PickerBuilder;
  setCallback(cb: (d: PickerData) => void): PickerBuilder;
  build(): { setVisible(v: boolean): void };
}
interface DocsView { setMimeTypes(m: string): DocsView; setIncludeFolders(v: boolean): DocsView }
interface PickerApi {
  PickerBuilder: new () => PickerBuilder;
  DocsView: new (viewId?: unknown) => DocsView;
  ViewId: { DOCS: unknown };
  Feature: { MULTISELECT_ENABLED: unknown; SUPPORT_DRIVES: unknown };
  Action: { PICKED: string; CANCEL: string };
}
interface GoogleGlobal { picker?: PickerApi }
interface Gapi { load(name: string, opts: { callback: () => void; onerror: () => void }): void }

let loading: Promise<PickerApi> | null = null;

/** Google's picker script, loaded once. */
function loadPicker(): Promise<PickerApi> {
  loading ??= new Promise<PickerApi>((resolve, reject) => {
    const failed = () => {
      loading = null;
      reject(new Error('Google’s file picker could not be loaded. Is this machine offline?'));
    };
    const s = document.createElement('script');
    s.src = 'https://apis.google.com/js/api.js';
    s.async = true;
    s.onerror = failed;
    s.onload = () => {
      const w = window as unknown as { gapi?: Gapi; google?: GoogleGlobal };
      if (!w.gapi) return failed();
      w.gapi.load('picker', {
        callback: () => (w.google?.picker ? resolve(w.google.picker) : failed()),
        onerror: failed,
      });
    };
    document.head.appendChild(s);
  });
  return loading;
}

/** Opens Google's picker as an account: the Docs and Sheets picked (none when cancelled). */
export async function pickFromDrive(connectionId: string): Promise<PickerDoc[]> {
  const cfg = await api.googlePicker(connectionId);
  const g = await loadPicker();
  return new Promise((resolve) => {
    const view = new g.DocsView(g.ViewId.DOCS)
      .setMimeTypes('application/vnd.google-apps.document,application/vnd.google-apps.spreadsheet')
      .setIncludeFolders(true);
    new g.PickerBuilder()
      .addView(view)
      .enableFeature(g.Feature.MULTISELECT_ENABLED)
      .enableFeature(g.Feature.SUPPORT_DRIVES)
      .setOAuthToken(cfg.accessToken)
      .setDeveloperKey(cfg.apiKey)
      .setAppId(cfg.appId)
      .setTitle('Pick Docs and Sheets for PDT')
      .setCallback((d) => {
        if (d.action === g.Action.PICKED) resolve(d.docs ?? []);
        else if (d.action === g.Action.CANCEL) resolve([]);
      })
      .build()
      .setVisible(true);
  });
}
