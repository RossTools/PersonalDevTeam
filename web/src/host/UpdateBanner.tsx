import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button } from '@ui';
import { api, ApiError } from '../api/client';

export function UpdateBanner() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus, refetchInterval: 60_000 });
  const refresh = () => qc.invalidateQueries({ queryKey: ['host-update'] });
  const install = useMutation({
    mutationFn: async () => {
      try {
        return await api.installUpdate(false);
      } catch (e) {
        // A busy host asks for confirmation; declining leaves it alone.
        if (e instanceof ApiError && e.code === 'host_busy') {
          if (window.confirm(e.message)) return api.installUpdate(true);
          return null;
        }
        throw e;
      }
    },
    onSettled: refresh,
  });
  const dismiss = useMutation({ mutationFn: api.dismissUpdateResult, onSettled: refresh });
  const s = status.data;
  if (!s) return null;
  const r = s.lastResult;
  return (
    <>
      {r && (
        <div role={r.outcome === 'installed' ? 'status' : 'alert'} className={`flex flex-wrap items-center gap-2 border-b border-border px-3 py-1 text-xs ${r.outcome === 'installed' ? 'text-running' : 'text-failed'}`}>
          <span>{r.message}</span>
          {r.outcome !== 'installed' && r.log && <code className="text-muted">{r.log}</code>}
          <Button variant="ghost" onClick={() => dismiss.mutate()}>Dismiss</Button>
        </div>
      )}
      {s.state === 'available' && s.latest && (
        <div className="flex flex-wrap items-center gap-2 border-b border-border bg-accent-soft px-3 py-1 text-xs">
          <span className="font-medium">Update to v{s.latest.version} available</span>
          {s.latest.notes && (
            <details className="text-muted">
              <summary>Release notes</summary>
              <pre className="whitespace-pre-wrap font-sans">{s.latest.notes}</pre>
            </details>
          )}
          <Button variant="primary" disabled={install.isPending} onClick={() => install.mutate()}>Update</Button>
          {install.isError && <span role="alert" className="text-failed">{install.error.message}</span>}
        </div>
      )}
      {(s.state === 'downloading' || s.state === 'installing') && (
        <div role="status" className="border-b border-border px-3 py-1 text-xs text-waiting">
          {s.state === 'downloading' ? 'Downloading the update…' : 'Installing the update; this page reconnects when the host is back.'}
        </div>
      )}
    </>
  );
}
