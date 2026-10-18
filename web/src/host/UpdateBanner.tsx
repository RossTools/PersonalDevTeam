import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button } from '@ui';
import { api, ApiError } from '../api/client';

export function UpdateBanner() {
  const qc = useQueryClient();
  const status = useQuery({
    queryKey: ['host-update'],
    queryFn: api.updateStatus,
    // While an update is underway, poll fast so the page reloads soon after the new version answers.
    refetchInterval: (q) => (q.state.data && BUSY.includes(q.state.data.state) ? 2_000 : 60_000),
  });
  useReloadOnNewVersion(status.data?.current);
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
    onSuccess: (next) => {
      if (next) qc.setQueryData(['host-update'], next);
    },
    onSettled: refresh,
  });
  const dismiss = useMutation({ mutationFn: api.dismissUpdateResult, onSettled: refresh });
  const [notesOpen, setNotesOpen] = useState(false);
  const s = status.data;
  if (!s) return null;
  const r = s.lastResult;
  // Every strip is one line of fixed height, and the release notes float over the page: the banner sits above the
  // terminal, so any change in its height resizes the PTY and makes the agent redraw (on a phone, a wrapped banner
  // with open notes left Claude one row).
  return (
    <>
      {r && (
        <div role={r.outcome === 'installed' ? 'status' : 'alert'} className={`${STRIP} ${r.outcome === 'installed' ? 'text-running' : 'text-failed'}`}>
          <span className="min-w-0 truncate" title={r.message}>{r.message}</span>
          {r.outcome !== 'installed' && r.log && <code className="min-w-0 truncate text-muted" title={r.log}>{r.log}</code>}
          <Button variant="ghost" className={SMALL} onClick={() => dismiss.mutate()}>Dismiss</Button>
        </div>
      )}
      {s.state === 'available' && s.latest && (
        <div className={`${STRIP} relative overflow-visible bg-accent-soft`}>
          <span className="min-w-0 truncate font-medium">Update to v{s.latest.version} available</span>
          {s.latest.notes && (
            <Button variant="ghost" className={SMALL} aria-expanded={notesOpen} onClick={() => setNotesOpen((o) => !o)}>
              Release notes
            </Button>
          )}
          <Button variant="primary" className={SMALL} disabled={install.isPending} onClick={() => install.mutate()}>Update</Button>
          {install.isError && <span role="alert" className="min-w-0 truncate text-failed" title={install.error.message}>{install.error.message}</span>}
          {notesOpen && s.latest.notes && (
            <div role="region" aria-label="Release notes" className="absolute inset-x-0 top-full z-20 max-h-[50vh] overflow-auto border-b border-border bg-surface p-3 shadow-lg">
              <pre className="whitespace-pre-wrap font-sans text-xs text-fg">{s.latest.notes}</pre>
              <Button variant="ghost" className={`${SMALL} mt-2`} onClick={() => setNotesOpen(false)}>Close</Button>
            </div>
          )}
        </div>
      )}
      {(s.state === 'downloading' || s.state === 'installing') && (
        <div role="status" className={`${STRIP} text-waiting`}>
          <span className="min-w-0 truncate">
            {s.state === 'downloading' ? 'Downloading the update…' : 'Installing the update; this page reloads when the host is back.'}
          </span>
        </div>
      )}
    </>
  );
}

const BUSY: string[] = ['downloading', 'installing'];

/** Reloads the page once pdtd reports a different version than the one this page loaded against (an update installed
 * from here, the tray or another device), so the browser never runs an old UI against a new daemon. */
function useReloadOnNewVersion(current: string | undefined) {
  const loaded = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!current) return;
    if (loaded.current === undefined) loaded.current = current;
    else if (current !== loaded.current) location.reload();
  }, [current]);
}

const STRIP = 'flex h-8 shrink-0 flex-nowrap items-center gap-2 overflow-hidden whitespace-nowrap border-b border-border px-3 text-xs';
const SMALL = 'shrink-0 px-2 py-0.5 text-xs';
