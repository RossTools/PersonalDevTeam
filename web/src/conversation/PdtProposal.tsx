// What a PDT tool waiting for the user's yes would create or change (PDT-382), for its approval card. Every asking
// tool has one, and it shows all the text the call would post or write, never a cut of it (PDT-541): what the card
// leaves out is what a prompt-injected call could hide.
import type { ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Badge, Chip } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { LabelChips } from '../common/LabelChips';
import { priorityLabel } from '../common/PriorityIcon';
import { StageBadge } from '../common/StageBadge';
import { stageLabel } from '../common/StageStepper';
import { TicketPill } from '../common/TicketPill';
import { ChangeRow, Key } from './PdtToolCard';
import { createdTicket, editText, LINK_TEXT, pdtApprovalTitle, pdtTool, sentMessage, ticketEdit, ticketLink } from './pdtTools';
import { isRecord, nonBlank, strings } from '../common/guards';
import { capitalize, plural } from '../common/text';
import { sessionKeys } from '../sessions/queries';
import { ticketKeys } from '../tickets/queries';
import { documentKeys } from '../documents/queries';

function Box({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-1.5 rounded-md border border-border bg-surface-2 p-2.5">{children}</div>;
}

/** Text the call carries, whole and as written (not rendered: Markdown could hide some of it). */
function Text({ label, children, mono = false }: { label?: string; children: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      {label && <span className="text-xs text-muted-foreground">{label}</span>}
      <pre className={cn('max-h-60 overflow-auto rounded-sm border border-border bg-card p-2 text-xs whitespace-pre-wrap break-words', mono ? 'font-mono' : 'font-sans')}>
        {children}
      </pre>
    </div>
  );
}

/** A short line: "In ALP", "Labels …". */
function Line({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">{children}</div>;
}

/** The ticket a call names (a key or an id), as its key and title; as given while (or when it can't be) read. */
function TicketRef({ value, none = 'a ticket' }: { value: string | null; none?: string }) {
  const t = useQuery({ queryKey: ticketKeys.card(value ?? ''), queryFn: () => api.getTicket(value!), enabled: !!value, staleTime: 30_000, retry: false });
  if (!value) return <span className="text-muted-foreground">{none}</span>;
  if (!t.data) return <Key value={value} />;
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <TicketPill ticketKey={t.data.key} />
      <span className="min-w-0 truncate text-foreground">{t.data.title}</span>
    </span>
  );
}

/** The document a call names (an id or its exact title), as its title. */
function DocName({ value }: { value: string | null }) {
  const d = useQuery({ queryKey: documentKeys.one(value ?? ''), queryFn: () => api.document(value!), enabled: !!value, staleTime: 30_000, retry: false });
  return <span className="font-semibold">{d.data?.title ?? value ?? 'A document'}</span>;
}

function SessionName({ id }: { id: string | null }) {
  const s = useQuery({ queryKey: sessionKeys.one(id ?? ''), queryFn: () => api.getSession(id!), enabled: !!id, staleTime: 60_000, retry: false });
  return <span className="font-medium">{s.data?.title ?? 'a session'}</span>;
}

/** Name → value pairs (a workflow's input, a command's environment). */
function Pairs({ entries, mono = false }: { entries: [string, unknown][]; mono?: boolean }) {
  return (
    <dl className={cn('grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs', mono && 'font-mono')}>
      {entries.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-muted-foreground">{k}</dt>
          <dd className="min-w-0 break-words">{typeof v === 'string' ? v : JSON.stringify(v)}</dd>
        </div>
      ))}
    </dl>
  );
}

function NewTicket({ input }: { input: unknown }) {
  const t = createdTicket(input);
  const a = isRecord(input) ? input : {};
  const parent = nonBlank(a.parent);
  const reason = nonBlank(a.reason);
  const files = Array.isArray(a.files) ? a.files.length : 0;
  const areas = strings(a.areas);
  const external = strings(a.external);
  return (
    <Box>
      <div className="font-semibold">{t.title}</div>
      {nonBlank(a.project) && <Line>In <Chip>{nonBlank(a.project)}</Chip></Line>}
      {t.description && <Text>{t.description}</Text>}
      <div className="flex flex-wrap items-center gap-1">
        <StageBadge stage={t.stage} />
        {t.priority && t.priority !== 'none' && <Badge>{priorityLabel(t.priority)}</Badge>}
        <LabelChips labels={t.labels} className="flex-wrap" />
        {parent && <span className="ml-1 flex items-center gap-1 text-xs text-muted-foreground">Part of <TicketRef value={parent} /></span>}
        {files > 0 && <span className="ml-1 text-xs text-muted-foreground">{plural(files, 'file')} attached</span>}
      </div>
      {areas.length > 0 && <Line>Areas: {areas.join(', ')}</Line>}
      {external.length > 0 && (
        <div className="flex flex-col gap-0.5 text-xs">
          <span className="text-muted-foreground">Links to</span>
          {external.map((e) => <span key={e} className="font-mono break-all">{e}</span>)}
        </div>
      )}
      {reason && <p className="text-xs"><span className="text-muted-foreground">Why: </span>{reason}</p>}
    </Box>
  );
}

/** What one document edit writes, under its phrase. */
function EditBody({ edit }: { edit: unknown }) {
  const e = isRecord(edit) ? edit : {};
  const text = nonBlank(e.markdown)
    ?? (typeof e.find === 'string' ? `${e.find}\n→\n${typeof e.replace === 'string' ? e.replace : ''}` : null)
    ?? (Array.isArray(e.cells) ? strings(e.cells).join(' | ') : null)
    ?? (typeof e.value === 'string' ? `${nonBlank(e.column) ?? '?'}, row ${String(e.row ?? '?')}: ${e.value}` : null)
    ?? (Array.isArray(e.values) ? e.values.map((r) => strings(r).join('\t')).join('\n') : null);
  const phrase = editText(edit);
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs">{capitalize(phrase)}</span>
      {text !== null && <Text mono>{text}</Text>}
    </div>
  );
}

/**
 * A PDT tool's approval, wherever it is asked (the conversation's card, the Inbox): its question ("Create this
 * ticket?") and what it would create or change. Null for a tool without one.
 */
export function pdtApproval(tool: string, input: unknown): { title: string; preview: ReactNode } | null {
  const title = pdtApprovalTitle(tool, input);
  return title ? { title, preview: <PdtProposal tool={tool} input={input} /> } : null;
}

/** The approval card's preview for a PDT tool (`pdtApprovalTitle` names the tools that have one). */
export function PdtProposal({ tool, input }: { tool: string; input: unknown }) {
  const a = isRecord(input) ? input : {};
  const ticket = nonBlank(a.ticket);
  const body = nonBlank(a.body);
  switch (pdtTool(tool)) {
    case 'create_ticket':
      return <NewTicket input={input} />;
    case 'edit_ticket': {
      const e = ticketEdit(input);
      const description = typeof a.description === 'string' ? a.description : null;
      return (
        <Box>
          <Line><TicketRef value={ticket} /></Line>
          <div className="flex flex-col divide-y divide-border">
            {e.changes.filter((c) => c.field !== 'description').map((c) => <ChangeRow key={c.field} c={c} />)}
            {description !== null && (
              <div className="py-2">
                {description.trim() ? <Text label="New description">{description}</Text> : <span className="text-xs">Description emptied</span>}
              </div>
            )}
            {e.files > 0 && <div className="py-2 text-xs text-muted-foreground">Attaches {plural(e.files, 'file')}</div>}
          </div>
        </Box>
      );
    }
    case 'link_tickets':
    case 'unlink_tickets': {
      const l = ticketLink(input);
      return (
        <Box>
          <div className="flex flex-wrap items-center gap-2">
            <TicketRef value={l.ticket} />
            <span>{LINK_TEXT[l.kind]}</span>
            <TicketRef value={l.other} />
          </div>
        </Box>
      );
    }
    case 'send_to_session': {
      const m = sentMessage(input);
      return (
        <Box>
          <div className="text-xs text-muted-foreground">To <SessionName id={m.session} /></div>
          <blockquote className="border-l-2 border-border pl-3 whitespace-pre-wrap">{m.text}</blockquote>
        </Box>
      );
    }
    case 'add_ticket_comment':
      return (
        <Box>
          <Line>On <TicketRef value={ticket} none="this session's ticket" /></Line>
          <Text>{body ?? ''}</Text>
          {strings(a.files).length > 0 && <Line>Attaches {strings(a.files).length === 1 ? 'a file' : `${strings(a.files).length} files`}</Line>}
        </Box>
      );
    case 'workflow_run': {
      const name = nonBlank(a.name) ?? nonBlank(a.id) ?? 'A workflow';
      const given = isRecord(a.input) ? Object.entries(a.input) : [];
      return (
        <Box>
          <div className="font-semibold">{name}</div>
          {nonBlank(a.project) && <div className="text-xs text-muted-foreground">In {nonBlank(a.project)}</div>}
          {given.length > 0 && <Pairs entries={given} />}
        </Box>
      );
    }
    case 'workflow_import':
      return (
        <Box>
          <Line>They come in turned off.</Line>
          <Text mono>{nonBlank(a.text) ?? ''}</Text>
        </Box>
      );
    case 'stop_ticket': {
      const given = nonBlank(a.stage);
      const stage = given ? stageLabel(given) : 'To do';
      return (
        <Box>
          <div className="flex flex-wrap items-center gap-1.5">
            <TicketRef value={ticket} />
            <span>stops and goes back to {stage}</span>
          </div>
          {a.removeWorktree === true && (
            <p className="text-xs text-failed">
              Its worktree is removed, uncommitted changes too{a.deleteBranch === true ? ', and its branch deleted' : ''}.
            </p>
          )}
        </Box>
      );
    }
    case 'issue_create':
      return (
        <Box>
          <Line>
            {ticket ? <>On the forge of <TicketRef value={ticket} />'s project, linked to it</>
              : nonBlank(a.project) ? <>On the forge of <Chip>{nonBlank(a.project)}</Chip></>
                : "On the forge of this session's project"}
          </Line>
          <div className="font-semibold">{nonBlank(a.title) ?? (ticket ? "The ticket's title" : 'Untitled')}</div>
          {body ? <Text>{body}</Text> : ticket && <Line>The ticket's description as its body</Line>}
          <LabelChips labels={strings(a.labels)} className="flex-wrap" />
        </Box>
      );
    case 'report_bug':
      return (
        <Box>
          <Line>Where Host settings send bug reports: an issue on PDT's repository, or a ticket here</Line>
          <div className="font-semibold">{nonBlank(a.title) ?? 'Untitled'}</div>
          {nonBlank(a.description) && <Text label="What happened">{nonBlank(a.description)!}</Text>}
          {nonBlank(a.expected) && <Text label="Expected">{nonBlank(a.expected)!}</Text>}
          {nonBlank(a.steps) && <Text label="Steps">{nonBlank(a.steps)!}</Text>}
          {a.includeDiagnostics === true && <Line>With this instance's diagnostics (version, OS, the end of its log), redacted</Line>}
        </Box>
      );
    case 'link_issue':
    case 'unlink_issue':
      return (
        <Box>
          <Line><TicketRef value={ticket} /></Line>
          <span className="font-mono text-xs break-all">{nonBlank(a.issue) ?? '?'}</span>
          {nonBlank(a.label) && <Line>Shown as “{nonBlank(a.label)}”</Line>}
          {a.closeOnDone === true && <Line>Closed when the ticket is done</Line>}
        </Box>
      );
    case 'document_create':
      return (
        <Box>
          <div className="font-semibold">{nonBlank(a.title) ?? 'Untitled'}</div>
          <Line>
            {a.kind === 'sheet' ? 'A spreadsheet' : 'A document'}
            {nonBlank(a.provider) && <> in {nonBlank(a.provider)}</>}
            {ticket ? <>, linked to <TicketRef value={ticket} /></> : nonBlank(a.project) && <>, in <Chip>{nonBlank(a.project)}</Chip></>}
          </Line>
          {nonBlank(a.markdown) && <Text>{nonBlank(a.markdown)!}</Text>}
        </Box>
      );
    case 'doc_edit':
    case 'sheet_edit': {
      const edits = Array.isArray(a.edits) ? a.edits : [];
      return (
        <Box>
          <DocName value={nonBlank(a.document)} />
          {edits.map((e, i) => <EditBody key={i} edit={e} />)}
        </Box>
      );
    }
    case 'document_link':
      return (
        <Box>
          <DocName value={nonBlank(a.document)} />
          <Line>
            {a.unlink === true ? 'No longer linked to' : 'Linked to'}{' '}
            {ticket ? <TicketRef value={ticket} /> : nonBlank(a.project) ? <Chip>{nonBlank(a.project)}</Chip> : "this session's ticket"}
          </Line>
        </Box>
      );
    case 'comment_reply':
    case 'comment_resolve':
      return (
        <Box>
          <Line>On a comment in <DocName value={nonBlank(a.document)} /></Line>
          {body && <Text>{body}</Text>}
        </Box>
      );
    case 'run_with_secrets': {
      const env = isRecord(a.env) ? Object.entries(a.env) : [];
      return (
        <Box>
          <Text mono>{nonBlank(a.command) ?? ''}</Text>
          {nonBlank(a.cwd) && <Line>In <span className="font-mono">{nonBlank(a.cwd)}</span></Line>}
          {env.length > 0 && <Pairs entries={env} mono />}
        </Box>
      );
    }
    case 'write_secret_file':
      return (
        <Box>
          <span className="font-mono text-xs break-all">{nonBlank(a.path) ?? '?'}</span>
          <Text mono>{nonBlank(a.template) ?? ''}</Text>
        </Box>
      );
    default:
      return null;
  }
}
