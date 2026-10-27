import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Banner, Button, Link, useConfirm } from '@ui';
import { api, ApiError } from '../api/client';
import type { ReleaseChanges } from '../api/gen/ReleaseChanges';
import { useOpenTicket } from '../tickets/actions';

// Banner buttons stay 24px on touch screens too: the strip is a fixed h-8 (a 44px button would be clipped, and growing
// the strip would resize the terminal under it).

export function UpdateBanner() {
  const qc = useQueryClient();
  const confirm = useConfirm();
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
          const { ok } = await confirm({ title: 'Update now?', body: e.message, action: 'Update anyway' });
          return ok ? api.installUpdate(true) : null;
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
  // Every change since the installed version when GitHub could list them, else the release's own notes.
  const changes = s.latest?.changes?.entries.length ? s.latest.changes : null;
  const hasNotes = !!(changes || s.latest?.notes);
  // Every strip is one line of fixed height, and the release notes float over the page: the banner sits above the
  // terminal, so any change in its height resizes the PTY and makes the agent redraw (on a phone, a wrapped banner
  // with open notes left Claude one row).
  return (
    <>
      {r && (
        <Banner
          tone={r.outcome === 'installed' ? 'running' : 'failed'}
          title={r.message}
          actions={<Button size="sm" variant="ghost" compact onClick={() => dismiss.mutate()}>Dismiss</Button>}
        >
          {r.message}
          {r.outcome !== 'installed' && r.log && <> · <code className="text-muted-foreground" title={r.log}>{r.log}</code></>}
          {dismiss.isError && <span role="alert" className="text-destructive" title={dismiss.error.message}> · {dismiss.error.message}</span>}
        </Banner>
      )}
      {s.state === 'available' && s.latest && (
        <div className="relative shrink-0">
          <Banner
            tone="accent"
            actions={
              <>
                {hasNotes && (
                  <Button size="sm" variant="ghost" compact aria-expanded={notesOpen} onClick={() => setNotesOpen((o) => !o)}>
                    Release notes
                  </Button>
                )}
                <Button size="sm" variant="primary" compact pending={install.isPending} onClick={() => install.mutate()}>Update</Button>
              </>
            }
          >
            Update to v{s.latest.version} available
            {install.isError && <span role="alert" className="text-destructive" title={install.error.message}> · {install.error.message}</span>}
          </Banner>
          {notesOpen && hasNotes && (
            <div
              role="region"
              aria-label="Release notes"
              className="absolute inset-x-0 top-full z-(--pdt-z-popover) max-h-[50vh] overflow-auto border-b border-border bg-popover p-3 shadow-popover"
            >
              {changes
                ? <ChangeList changes={changes} onOpened={() => setNotesOpen(false)} />
                : <pre className="font-sans text-xs whitespace-pre-wrap text-foreground">{s.latest.notes}</pre>}
              <Button size="sm" variant="ghost" className="mt-2" onClick={() => setNotesOpen(false)}>Close</Button>
            </div>
          )}
        </div>
      )}
      {(s.state === 'downloading' || s.state === 'installing') && (
        <Banner tone="waiting">
          {s.state === 'downloading' ? 'Downloading the update…' : 'Installing the update; this page reloads when the host is back.'}
        </Banner>
      )}
    </>
  );
}

const BUSY: string[] = ['downloading', 'installing'];

/** One line per change (a squashed pull request), newest first: its ticket here and its pull request on GitHub. */
function ChangeList({ changes, onOpened }: { changes: ReleaseChanges; onOpened: () => void }) {
  const openTicket = useOpenTicket();
  const { entries, total, since, moreUrl } = changes;
  return (
    <div className="text-xs text-foreground">
      <p className="font-medium">{total === 1 ? '1 change' : `${total} changes`} since v{since}</p>
      <ul className="mt-1 space-y-0.5">
        {entries.map((e) => {
          const here = e.ticketHere;
          return (
            <li key={e.sha}>
              {e.ticket && (
                <>
                  {here
                    ? <Link onClick={() => { openTicket(here); onOpened(); }}>{e.ticket}</Link>
                    : <span className="text-muted-foreground">{e.ticket}</span>}
                  {' '}
                </>
              )}
              {e.title}
              {e.prUrl && <> <Link external href={e.prUrl}>#{e.pr}</Link></>}
            </li>
          );
        })}
      </ul>
      {moreUrl && <Link external href={moreUrl} className="mt-1">…and {total - entries.length} more</Link>}
    </div>
  );
}

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

