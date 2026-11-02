import { useEffect, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Badge, Banner, Button, Link, Popover, useConfirm } from '@ui';
import { api, ApiError } from '../api/client';
import type { ReleaseChanges } from '../api/gen/ReleaseChanges';
import type { ReleaseNotes } from '../api/gen/ReleaseNotes';
import { Markdown } from '../common/Markdown';
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
    meta: { errorShown: true },
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
  const dismiss = useMutation({ meta: { errorShown: true }, mutationFn: api.dismissUpdateResult, onSettled: refresh });
  const s = status.data;
  if (!s) return null;
  const r = s.lastResult;
  // The notes of every release since the installed version when GitHub could list them, else every commit since
  // it, else the new release's own notes.
  const releases = s.latest?.releases?.length ? s.latest.releases : null;
  const changes = s.latest?.changes?.entries.length ? s.latest.changes : null;
  const hasNotes = !!(releases || changes || s.latest?.notes);
  // Every strip is one line of fixed height, and the release notes open in a popover over the page: the banner sits
  // above the terminal, so any change in its height resizes the PTY and makes the agent redraw (on a phone, a wrapped
  // banner with open notes left Claude one row).
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
        <Banner
          tone="accent"
          actions={
            <>
              {hasNotes && (
                <Popover
                  trigger={<Button size="sm" variant="ghost" compact>Release notes</Button>}
                  label="Release notes"
                  placement="bottom end"
                  className="w-[min(40rem,calc(100vw-2rem))] p-3"
                >
                  {({ close }) => (
                    <>
                      {/* React Aria sets the popover's own max height inline; the notes keep to half the screen. */}
                      <div className="max-h-[50vh] min-h-0 overflow-y-auto">
                        {releases
                          ? <ReleaseList releases={releases} since={s.current} />
                          : changes
                          ? <ChangeList changes={changes} onOpened={close} />
                          : <pre className="font-sans text-xs whitespace-pre-wrap text-foreground">{s.latest?.notes}</pre>}
                      </div>
                      <Button size="sm" variant="ghost" className="self-start" onClick={close}>Close</Button>
                    </>
                  )}
                </Popover>
              )}
              <Button size="sm" variant="primary" compact pending={install.isPending} onClick={() => install.mutate()}>Update</Button>
            </>
          }
        >
          Update to v{s.latest.version} available
          {s.latest.nightly && <> <Badge>Nightly</Badge></>}
          {install.isError && <span role="alert" className="text-destructive" title={install.error.message}> · {install.error.message}</span>}
        </Banner>
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

/** Each release the update brings, newest first, with its notes (written for people who use PDT, in Markdown). */
function ReleaseList({ releases, since }: { releases: ReleaseNotes[]; since: string }) {
  return (
    <div className="space-y-3 text-xs text-foreground">
      {releases.length > 1 && <p className="font-medium">{releases.length} releases since v{since.replace(/\+.*$/, '').replace(/-local$/, '')}</p>}
      {releases.map((r) => (
        <section key={r.tag} aria-label={`v${r.version}`}>
          <h3 className="flex items-center gap-2 font-medium">v{r.version} {r.nightly && <Badge>Nightly</Badge>}</h3>
          {r.notes.trim()
            ? <Markdown className="text-xs">{r.notes}</Markdown>
            : <p className="text-muted-foreground">No notes.</p>}
        </section>
      ))}
    </div>
  );
}

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

