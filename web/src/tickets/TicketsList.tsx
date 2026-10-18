import { useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Button, Dialog, Field, IconButton, Input, List, ListRow, useMenuAt } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import { ProjectKey } from '../common/BranchChip';
import { OPEN_STAGES } from './model';
import { ticketKeys } from './queries';
import type { BacklogState } from './useBacklog';

const listHeading = 'flex items-center px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/** Open tickets per project: one `limit=1` list request each (the page's `total`), refreshed every 10 s. */
function useOpenCounts(projects: Project[]) {
  const qs = useQueries({
    queries: projects.map((p) => ({
      queryKey: ticketKeys.count(p.id),
      queryFn: () => api.listTickets({ project: p.id, stage: OPEN_STAGES.join(','), limit: 1 }).then((r) => r.total),
      refetchInterval: 10_000,
    })),
  });
  return new Map(projects.map((p, i) => [p.id, qs[i]?.data]));
}

function SaveFilterDialog({ onSave, onClose }: { onSave: (name: string) => void; onClose: () => void }) {
  const [name, setName] = useState('');
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title="Save filter"
      description="Saves the project and filters on screen, in this browser."
      size="sm"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          <Button type="submit" form="save-filter" variant="primary" disabled={!name.trim()}>Save</Button>
        </>
      }
    >
      <form id="save-filter" onSubmit={(e) => { e.preventDefault(); if (name.trim()) { onSave(name.trim()); onClose(); } }}>
        <Field label="Name">
          <Input autoFocus value={name} maxLength={60} onChange={(e) => setName(e.target.value)} placeholder="Ready, high priority" />
        </Field>
      </form>
    </Dialog>
  );
}

/** The Tickets view's list column: All projects, each project with its key and open-ticket count, saved filters. */
export function TicketsList({ backlog, projects, onChoose }: { backlog: BacklogState; projects: Project[]; onChoose?: () => void }) {
  const counts = useOpenCounts(projects);
  const [saving, setSaving] = useState(false);
  const { openMenu, menu } = useMenuAt();
  return (
    <nav aria-label="Ticket lists" className="flex flex-col gap-0.5 py-1">
      <h2 className="px-3 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase">Tickets</h2>
      <List aria-label="Projects" onAction={(id) => { backlog.setProjectId(id === 'all' ? null : id); onChoose?.(); }}>
        <ListRow id="all" title="All projects" current={backlog.projectId === null} />
        {projects.map((p) => {
          const n = counts.get(p.id);
          return (
            <ListRow
              key={p.id}
              id={p.id}
              title={p.name}
              textValue={p.name}
              current={backlog.projectId === p.id}
              leading={<ProjectKey projectKey={p.key} />}
              trailing={n !== undefined && <span className="text-meta text-muted-foreground tabular-nums" aria-label={`${n} open`}>{n}</span>}
            />
          );
        })}
      </List>
      <h3 className={listHeading}>
        <span className="flex-1">Saved filters</span>
        <IconButton label="Save filter…" size="sm" noTooltip className="size-5" onClick={() => setSaving(true)}>
          <Plus aria-hidden />
        </IconButton>
      </h3>
      {/* Never empty: the built-in "Ready to start" is always first. */}
      <List
        aria-label="Saved filters"
        items={backlog.saved}
        onAction={(id) => {
          const f = backlog.saved.find((x) => x.id === id);
          if (f) { backlog.applyFilter(f); onChoose?.(); }
        }}
      >
        {(f) => (
          <ListRow
            id={f.id}
            title={f.name}
            secondary={
              f.builtin
                ? 'Ready, unblocked, no overlap'
                : f.projectId ? projects.find((p) => p.id === f.projectId)?.name ?? 'Removed project' : 'All projects'
            }
            onMenu={(at) => openMenu(at, `Saved filter ${f.name}`, [
              { label: 'Apply', onSelect: () => backlog.applyFilter(f) },
              ...(f.builtin ? [] : [{ label: 'Delete', danger: true, onSelect: () => backlog.deleteFilter(f.id) }]),
            ])}
          />
        )}
      </List>
      {menu}
      {saving && <SaveFilterDialog onSave={backlog.saveFilter} onClose={() => setSaving(false)} />}
    </nav>
  );
}
