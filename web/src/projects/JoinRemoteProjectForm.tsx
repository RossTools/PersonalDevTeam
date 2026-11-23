import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useState, type FormEvent } from 'react';
import { Alert, Button, Field, Input } from '@ui';
import { api } from '../api/client';
import type { RemoteProjectView } from '../api/gen/RemoteProjectView';
import { useHostSettings } from '../host/queries';
import { projectKeys } from './queries';

/**
 * Add project → Remote project: joins a project another instance owns, with the owner's address and the pairing code
 * it shows (its Project Settings → Remote instances → Add). The owner reaches this instance through remote access.
 */
export function JoinRemoteProjectForm({ onJoined, autoFocus = false }: { onJoined?: (v: RemoteProjectView) => void; autoFocus?: boolean }) {
  const qc = useQueryClient();
  const errorId = useId();
  const host = useHostSettings();
  const [owner, setOwner] = useState('');
  const [code, setCode] = useState('');
  const join = useMutation({
    meta: { errorShown: true },
    mutationFn: api.joinRemoteProject,
    onSuccess: (v) => {
      setOwner('');
      setCode('');
      void qc.invalidateQueries({ queryKey: projectKeys.remote });
      onJoined?.(v);
    },
  });
  useEffect(() => {
    if (join.isError) document.getElementById(errorId)?.focus();
  }, [join.isError, join.error, errorId]);
  const remote = host.data?.remote;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    join.mutate({ ownerAddress: owner, code });
  };
  return (
    <form onSubmit={submit} aria-label="Join a remote project" className="flex flex-col gap-3">
      <Field label="Owner's address" hint="The other instance's tailnet name, e.g. studio.example.ts.net.">
        <Input required mono autoFocus={autoFocus} placeholder="studio.example.ts.net" value={owner} onChange={(e) => setOwner(e.target.value)} />
      </Field>
      <Field label="Pairing code" hint="Shown on the owner under the project's settings → Remote instances → Add.">
        <Input required mono className="uppercase placeholder:normal-case" maxLength={9} placeholder="ABCD-EFGH" value={code} onChange={(e) => setCode(e.target.value)} />
      </Field>
      {remote && !remote.active && (
        <Alert tone="info">The owner reaches this instance through remote access: turn it on in Settings → This host → Remote access first.</Alert>
      )}
      {join.isError && <Alert id={errorId} tabIndex={-1} tone="error">{join.error.message}</Alert>}
      <Button variant="primary" type="submit" pending={join.isPending}>{join.isPending ? 'Pairing…' : 'Join project'}</Button>
    </form>
  );
}
