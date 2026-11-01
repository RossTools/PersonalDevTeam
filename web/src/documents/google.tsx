// The Google plugin in the browser (PDT-316): signing each Google account in (Google's consent page in a window of
// its own, which comes back to PDT and closes), and Google's file picker, which grants PDT the files the user picks.
import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Section, Skeleton } from '@ui';
import { api } from '../api/client';
import type { GoogleAccount } from '../api/gen/GoogleAccount';
import { actionError } from '../common/useAction';

export const googleAccountsKey = ['google', 'accounts'] as const;

/** The Google accounts and their sign-in state; read again quickly while a sign-in is under way. */
export function useGoogleAccounts(waiting = false, enabled = true) {
  return useQuery({ queryKey: googleAccountsKey, queryFn: api.googleAccounts, refetchInterval: waiting ? 2000 : false, enabled });
}

/** The address Google sends the browser back to: it must be one of the OAuth client's redirect URIs. */
export const redirectUri = () => `${window.location.origin}/api/google/callback`;

/** Each Google account's sign-in, on the Google plugin's page. */
export function GoogleSignIn() {
  const qc = useQueryClient();
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const accounts = useGoogleAccounts(waiting);
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
  const signIn = async (a: GoogleAccount) => {
    setError(null);
    // Opened now, in the click, so the browser doesn't block it; it goes to Google once PDT says where.
    const w = window.open('', 'pdt-google-sign-in', 'popup,width=520,height=680');
    try {
      const { url } = await api.googleSignIn(a.connectionId, window.location.origin);
      if (w) w.location.href = url;
      else window.location.assign(url);
      setWaiting(true);
    } catch (e) {
      w?.close();
      setError(actionError(e));
    }
  };
  const signOut = async (a: GoogleAccount) => {
    setError(null);
    try {
      await api.googleSignOut(a.connectionId);
      void qc.invalidateQueries({ queryKey: googleAccountsKey });
    } catch (e) {
      setError(actionError(e));
    }
  };
  return (
    <Section
      label="Sign in"
      description={<>Each account signs in with Google. Add <code className="text-foreground">{redirectUri()}</code> to your OAuth client’s authorized redirect URIs first.</>}
    >
      {accounts.isPending ? <Skeleton label="Loading Google accounts" className="h-8" />
        : accounts.isError ? <Alert tone="error">Google accounts could not be loaded: {accounts.error.message}</Alert>
          : accounts.data.length === 0 ? <p className="text-sm text-muted-foreground">Add a connection for your Google account first.</p>
            : accounts.data.map((a) => (
              <div key={a.connectionId} className="flex flex-wrap items-center gap-2 py-1">
                <span className="min-w-0 truncate text-sm font-medium">{a.name}</span>
                {a.signedIn
                  ? <span className="min-w-0 truncate text-xs text-muted-foreground">Signed in{a.email ? ` as ${a.email}` : ''}</span>
                  : <Badge tone="waiting">Not signed in</Badge>}
                {!a.picker && <span className="text-xs text-muted-foreground">No file picker (needs an API key and project number)</span>}
                <span className="flex-1" />
                {a.signedIn && <Button size="sm" variant="secondary" onClick={() => void signOut(a)}>Sign out</Button>}
                <Button size="sm" variant={a.signedIn ? 'secondary' : 'primary'} disabled={!a.hasClient} onClick={() => void signIn(a)}>
                  {a.signedIn ? 'Sign in again' : 'Sign in with Google'}
                </Button>
              </div>
            ))}
      {waiting && <p role="status" className="text-xs text-muted-foreground">Finish signing in in the Google window.</p>}
      {error && <Alert tone="error">{error}</Alert>}
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
