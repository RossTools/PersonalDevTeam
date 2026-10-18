import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Banner, Button, useConfirm } from '@ui';
import { api, ApiError } from '../api/client';

// Banner buttons stay 24px on touch screens too: the strip is a fixed h-8 (a 44px button would be clipped, and growing
// the strip would resize the terminal under it).
const SMALL = 'pointer-coarse:h-control-sm! pointer-coarse:px-2!';

export function UpdateBanner() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const status = useQuery({ queryKey: ['host-update'], queryFn: api.updateStatus, refetchInterval: 60_000 });
  const refresh = () => qc.invalidateQueries({ queryKey: ['host-update'] });
  const install = useMutation({
    mutationFn: async () => {
      try {
        return await api.installUpdate(false);
      } catch (e) {
        // A busy host asks for confirmation; declining leaves it alone.
        if (e instanceof ApiError && e.code === 'host_busy') {
          const { ok } = await confirm({ title: 'Update now?', body: e.message, action: 'Update anyway' });
          return ok ? api.installUpdate(true) : null;
        }
        throw e;
      }
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
        <Banner
          tone={r.outcome === 'installed' ? 'running' : 'failed'}
          title={r.message}
          actions={<Button size="sm" variant="ghost" className={SMALL} onClick={() => dismiss.mutate()}>Dismiss</Button>}
        >
          {r.message}
          {r.outcome !== 'installed' && r.log && <> · <code className="text-muted-foreground" title={r.log}>{r.log}</code></>}
        </Banner>
      )}
      {s.state === 'available' && s.latest && (
        <div className="relative shrink-0">
          <Banner
            tone="accent"
            actions={
              <>
                {s.latest.notes && (
                  <Button size="sm" variant="ghost" className={SMALL} aria-expanded={notesOpen} onClick={() => setNotesOpen((o) => !o)}>
                    Release notes
                  </Button>
                )}
                <Button size="sm" variant="primary" className={SMALL} pending={install.isPending} onClick={() => install.mutate()}>Update</Button>
              </>
            }
          >
            Update to v{s.latest.version} available
            {install.isError && <span role="alert" className="text-destructive" title={install.error.message}> · {install.error.message}</span>}
          </Banner>
          {notesOpen && s.latest.notes && (
            <div
              role="region"
              aria-label="Release notes"
              className="absolute inset-x-0 top-full z-(--pdt-z-popover) max-h-[50vh] overflow-auto border-b border-border bg-popover p-3 shadow-popover"
            >
              <pre className="font-sans text-xs whitespace-pre-wrap text-foreground">{s.latest.notes}</pre>
              <Button size="sm" variant="ghost" className="mt-2" onClick={() => setNotesOpen(false)}>Close</Button>
            </div>
          )}
        </div>
      )}
      {(s.state === 'downloading' || s.state === 'installing') && (
        <Banner tone="waiting">
          {s.state === 'downloading' ? 'Downloading the update…' : 'Installing the update; this page reconnects when the host is back.'}
        </Banner>
      )}
    </>
  );
}
