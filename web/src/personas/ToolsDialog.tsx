import { useState } from 'react';
import { Button, Checkbox, Dialog, EmptyState, Input, SectionLabel } from '@ui';
import type { PersonaTool } from '../api/gen/PersonaTool';
import { TOOL_GROUPS, toolGroup } from './model';

/** A row of the dialog: one of the server's tools, or a name the persona has that the server doesn't list. */
type Row = { name: string; does: string };

/**
 * Choose tools…: the server's tools by group, each group with an All box and how many are on, a search over names and
 * descriptions. A tick changes the form at once; the page's Save saves it.
 */
export function ToolsDialog({ known, checked, onToggle, onClose }: {
  known: PersonaTool[];
  checked: string[];
  /** Turns `names` on or off. */
  onToggle: (names: string[], on: boolean) => void;
  onClose: () => void;
}) {
  const [q, setQ] = useState('');
  const query = q.trim().toLowerCase();
  const rows: Row[] = known.map((t) => ({ name: t.name, does: `${t.does}${t.asks ? ' (asks you first)' : ''}` }));
  const unknown = checked.filter((n) => !known.some((t) => t.name === n)).map((name) => ({ name, does: 'Not a tool this PDT has' }));
  const groups = [
    ...TOOL_GROUPS.map((label) => ({ label, rows: rows.filter((r) => toolGroup(r.name) === label) })),
    { label: 'Not in this PDT', rows: unknown },
  ]
    .map((g) => ({ ...g, shown: g.rows.filter((r) => !query || r.name.includes(query) || r.does.toLowerCase().includes(query)) }))
    .filter((g) => g.shown.length > 0);
  return (
    <Dialog
      onClose={onClose}
      size="lg"
      title="Choose tools"
      headerActions={
        <Input type="search" size="sm" aria-label="Search tools" placeholder="Search tools" value={q} onChange={(e) => setQ(e.target.value)} className="flex-1" />
      }
      footer={<Button variant="primary" onClick={onClose}>Done</Button>}
    >
      {groups.length === 0 ? (
        <EmptyState size="inline">No tools match.</EmptyState>
      ) : (
        <div className="flex flex-col gap-4">
          {groups.map((g) => {
            const on = g.rows.filter((r) => checked.includes(r.name)).length;
            const shownOn = g.shown.filter((r) => checked.includes(r.name)).length;
            return (
              <section key={g.label} aria-label={g.label} className="flex flex-col gap-1.5">
                <div className="flex items-center gap-3">
                  <SectionLabel>{g.label}</SectionLabel>
                  <span className="text-meta tabular-nums text-muted-foreground">{on} of {g.rows.length} on</span>
                  <Checkbox
                    className="ml-auto"
                    aria-label={`All ${g.label}`}
                    checked={shownOn === g.shown.length}
                    indeterminate={shownOn > 0 && shownOn < g.shown.length}
                    onChange={(all) => onToggle(g.shown.map((r) => r.name), all)}
                  >
                    <span className="text-xs">All</span>
                  </Checkbox>
                </div>
                <ul className="flex flex-col gap-1.5">
                  {g.shown.map((r) => (
                    <li key={r.name}>
                      <Checkbox checked={checked.includes(r.name)} onChange={(o) => onToggle([r.name], o)}>
                        <span className="text-xs">
                          {r.does} <span className="font-mono text-meta text-muted-foreground">{r.name}</span>
                        </span>
                      </Checkbox>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      )}
    </Dialog>
  );
}
