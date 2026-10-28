import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Alert, Badge, Button, Input, Link, Section, SettingsRow, Skeleton, useConfirm } from '@ui';
import { api } from '../api/client';
import type { GithubLoginView } from '../api/gen/GithubLoginView';

const GITHUB_API = 'https://api.github.com';
const DESCRIPTION = 'The login PDT reads pull requests, CI and issues with, and merges and comments as.';

/**
 * Settings › This host › GitHub (PDT-177): the login PDT reads pull requests, CI and issues with, merges and comments
 * as. Connect GitHub (the device flow: enter a code on GitHub), or paste a token; a GitHub Enterprise Server by its API URL.
 */
export function GithubLogin() {
  const qc = useQueryClient();
  const view = useQuery({
    queryKey: ['github-login'],
    queryFn: api.githubLogin,
    // While a code waits to be entered, look often: PDT signs in as soon as GitHub says so.
    refetchInterval: (q) => (q.state.data?.pending ? 2000 : false),
  });
  const set = (v: GithubLoginView) => qc.setQueryData(['github-login'], v);
  const [token, setToken] = useState('');
  const [apiUrl, setApiUrl] = useState('');
  const [clientId, setClientId] = useState('');
  const [enterprise, setEnterprise] = useState(false);
  const enterpriseUrl = enterprise && apiUrl.trim() ? apiUrl.trim() : undefined;
  const device = useMutation({
    mutationFn: () => api.githubLoginDevice({ apiUrl: enterpriseUrl, clientId: enterprise && clientId.trim() ? clientId.trim() : undefined }),
    onSuccess: set,
  });
  const paste = useMutation({
    mutationFn: () => api.githubLoginToken(token.trim(), enterpriseUrl),
    onSuccess: (v) => { set(v); setToken(''); },
  });
  const cancel = useMutation({ mutationFn: api.githubCancelDevice, onSuccess: set });
  const disconnect = useMutation({ mutationFn: api.githubDisconnect, onSuccess: set });
  const confirm = useConfirm();
  const v = view.data;
  if (!v) {
    return (
      <Section label="GitHub" description={DESCRIPTION}>
        {view.error ? <Alert tone="error">The GitHub login could not be read: {view.error.message}</Alert> : <Skeleton label="Loading the GitHub login" className="h-16" />}
      </Section>
    );
  }

  const onDisconnect = async () => {
    const { ok } = await confirm({
      title: 'Disconnect GitHub?',
      body: 'PDT stops following pull requests and CI, merging and commenting until you connect again. The login leaves the keychain; revoke the app on GitHub too if you like.',
      action: 'Disconnect',
      danger: true,
    });
    if (ok) disconnect.mutate();
  };
  const onPaste = (e: FormEvent) => {
    e.preventDefault();
    if (token.trim()) paste.mutate();
  };
  const error = device.error ?? paste.error ?? disconnect.error;

  return (
    <Section label="GitHub" description={DESCRIPTION}>
      {v.connected && v.rejectedAt != null && (
        <Alert tone="error" title="GitHub no longer accepts this login.">
          The token was revoked or expired. PDT has stopped calling GitHub until you connect again: reconnect below, or
          paste a new token.
        </Alert>
      )}
      {v.connected ? (
        <SettingsRow
          label={<span className="flex items-center gap-2">Connected <Badge tone="accent">{v.login}</Badge></span>}
          description={v.apiUrl === GITHUB_API ? 'On github.com.' : `On ${v.webUrl}.`}
        >
          <Button size="sm" variant="danger" pending={disconnect.isPending} onClick={() => void onDisconnect()}>Disconnect…</Button>
        </SettingsRow>
      ) : (
        <Alert tone="warning" title="GitHub is not connected.">
          Until it is, PDT can’t follow pull requests or CI, merge, or let agents read CI logs and comment.
        </Alert>
      )}
      {v.pending ? (
        <SettingsRow
          label={<span>Enter <code className="font-mono text-base font-semibold">{v.pending.userCode}</code> on GitHub</span>}
          description={<>Open <Link href={v.pending.verificationUri} external>{v.pending.verificationUri}</Link>, enter the code and authorize PDT. This page updates once you have.</>}
        >
          <Button size="sm" pending={cancel.isPending} onClick={() => cancel.mutate()}>Cancel</Button>
        </SettingsRow>
      ) : (
        (v.deviceFlow || (enterprise && clientId.trim() !== '')) && (
          <SettingsRow
            label={v.connected ? 'Connect again' : 'Connect GitHub'}
            description={`Sign in on GitHub with a code. PDT asks for the ${v.scopes} scope.`}
          >
            <Button size="sm" variant={v.connected && v.rejectedAt == null ? 'secondary' : 'primary'} pending={device.isPending} onClick={() => device.mutate()}>
              {v.rejectedAt != null ? 'Reconnect' : 'Connect GitHub'}
            </Button>
          </SettingsRow>
        )
      )}
      {v.error && <Alert tone="error" title="The login failed.">{v.error}</Alert>}
      <form onSubmit={onPaste} className="flex flex-col gap-2">
        <SettingsRow
          label="Or paste a token"
          description={
            <>
              A token with the <code>{v.scopes}</code> scope (a fine-grained one needs read and write on pull requests, issues
              and contents, and read on actions). With the GitHub CLI, <code>gh auth token</code> prints one. Kept in the OS keychain.
            </>
          }
        >
          <Input aria-label="GitHub login token" type="password" autoComplete="off" className="w-56" value={token} onChange={(e) => setToken(e.target.value)} placeholder="gho_… or github_pat_…" />
          <Button type="submit" size="sm" disabled={!token.trim()} pending={paste.isPending}>Save login token</Button>
        </SettingsRow>
      </form>
      <SettingsRow label="GitHub Enterprise Server" description="Sign in to your own GitHub host instead of github.com.">
        <Button size="sm" variant="ghost" onClick={() => setEnterprise(!enterprise)}>{enterprise ? 'Use github.com' : 'Use an Enterprise host'}</Button>
      </SettingsRow>
      {enterprise && (
        <>
          <SettingsRow label="API URL" description="Its REST API, e.g. https://github.example.com/api/v3.">
            <Input aria-label="GitHub API URL" className="w-72" value={apiUrl} onChange={(e) => setApiUrl(e.target.value)} placeholder="https://github.example.com/api/v3" />
          </SettingsRow>
          <SettingsRow label="OAuth App client ID" description="An OAuth App registered on that host, with device flow on; PDT’s own works on github.com only.">
            <Input aria-label="Enterprise OAuth App client ID" className="w-72" value={clientId} onChange={(e) => setClientId(e.target.value)} />
          </SettingsRow>
        </>
      )}
      {error && <Alert tone="error">{error.message}</Alert>}
    </Section>
  );
}
