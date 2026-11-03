import { useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Checkbox, Dialog, Field, Input, SegmentedControl, Select, Skeleton, Switch } from '@ui';
import { api, ApiError } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { SessionMode } from '../api/gen/SessionMode';
import type { StartMode } from '../api/gen/StartMode';
import type { StartResult } from '../api/gen/StartResult';
import type { StartTicket } from '../api/gen/StartTicket';
import type { TabPlacement } from '../api/gen/TabPlacement';
import type { Ticket } from '../api/gen/Ticket';
import { TicketKey } from '../common/TicketKey';
import { actionError } from '../common/useAction';
import { PermissionModeSelect } from '../conversation/ConversationComposer';
import { useAgentAvailability } from '../common/useAgentAvailability';
import { isAcp, useAgents } from '../common/agents';
import { useAgentDefaults } from '../common/useAgentDefaults';
import { INITIAL_SIZE } from '../sessions/status';
import { isClosed, linkRole } from './model';
import { setWaiting } from './actions';
import { AreaChips } from './Areas';
import { useProjectAreas, useTicket } from './queries';
import { SharedFiles } from './TicketParts';
import { useProjects } from '../projects/queries';


/** The tickets that block this one and aren't Done or Won't do. */
export const openBlockers = (t: Pick<Ticket, 'links'>) => t.links.filter((l) => linkRole(l) === 'blockedBy' && !isClosed(l.stage));

export interface StartDialogProps {
  ticketId: string;
  /** Where the ticket and session (if any) tabs open; the end of the project workspace when absent. */
  placement?: TabPlacement;
  onClose: () => void;
  onStarted: (r: StartResult) => void;
}

/**
 * Start a ticket: agent (or no session), branch, base (neither for a Personal project), permission prompts, and where the tabs open. Open blockers and overlaps with
 * active tickets are listed first; then the primary action is **Wait** (wait until clear) and **Start anyway**
 * (`mode: force`; the danger variant for a conflict) is the other. The server's own check (`start_blocked`) does the
 * same.
 */
export function StartDialog({ ticketId, placement, onClose, onStarted }: StartDialogProps) {
  const q = useTicket(ticketId);
  const qc = useQueryClient();
  const projects = useProjects();
  const t = q.data;
  const areaConfig = useProjectAreas(t?.projectId ?? null);
  const config = areaConfig.data && !areaConfig.data.error ? areaConfig.data.areas : undefined;
  const project = projects.data?.find((p) => p.id === t?.projectId) ?? null;
  // A Personal project has no repository: the attempt works in the ticket's folder, with no branch or worktree.
  const personal = project?.kind === 'personal';
  // The host's default agent and view until this dialog picks others.
  const defaults = useAgentDefaults();
  const [pickedAgent, setAgent] = useState<AgentKind | null>(null);
  const chosenAgent = pickedAgent ?? defaults.agent;
  const availability = useAgentAvailability();
  const agents = useAgents();
  // A host with none of them still lists them all (starting one then says what is missing).
  const offeredAgents = agents.all.some(availability.offered) ? agents.all.filter(availability.offered) : agents.all;
  // The chosen agent unless this host lacks it (then the first it has); every agent until the checks load.
  const agent: AgentKind = offeredAgents.includes(chosenAgent) ? chosenAgent : offeredAgents[0] ?? chosenAgent;
  // 'none': no agent session (PDT-72), only the branch, worktree and the ticket's tab.
  const [pickedMode, setMode] = useState<SessionMode | 'none' | null>(null);
  // An ACP agent has only its conversation.
  const view = pickedMode === 'terminal' && isAcp(agent) ? 'conversation' : pickedMode ?? (isAcp(agent) && defaults.mode === 'terminal' ? 'conversation' : defaults.mode);
  const noSession = view === 'none';
  const sessionMode: SessionMode = isAcp(agent) ? 'conversation' : view === 'none' ? defaults.mode : view;
  const [branch, setBranch] = useState('');
  const [base, setBase] = useState('');
  // The host's default for tickets (Bypass unless changed) until this dialog picks another.
  const [pickedPermission, setPermission] = useState<PermissionMode | null>(null);
  const permissionMode = pickedPermission ?? defaults.ticketPermissionMode;
  const [newWorkspace, setNewWorkspace] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // The server answered start_blocked: its message, and Start becomes Start anyway.
  const [serverBlocked, setServerBlocked] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const errorRef = useRef<HTMLDivElement>(null);

  const blockers = t ? openBlockers(t) : [];
  const overlaps = t?.overlaps ?? [];
  const conflict = overlaps.some((o) => o.level === 'conflict');
  // Something stands in the way: Wait or Start anyway instead of Start.
  const force = blockers.length > 0 || overlaps.length > 0 || serverBlocked !== null;
  const waiting = !!t?.waiting;
  const active = t?.attempt?.state === 'active';
  const closed = t ? isClosed(t.stage) : false;
  const startable = !!t && !active && !closed;

  const wait = async () => {
    if (!t || !startable || pending) return;
    setPending(true);
    setError(null);
    try {
      await setWaiting(qc, t, true);
      onClose();
    } catch (e) {
      setError(actionError(e));
      requestAnimationFrame(() => errorRef.current?.focus());
    } finally {
      setPending(false);
    }
  };

  const stopWaiting = async () => {
    if (!t || pending) return;
    setPending(true);
    setError(null);
    try {
      await setWaiting(qc, t, false);
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };

  // Enter does the primary action: Wait when something is in the way (and it isn't waiting already), else Start.
  const offerWait = force && !waiting;
  const primary = () => void (offerWait ? wait() : submit());

  const submit = async () => {
    if (!t || !startable || pending) return;
    setPending(true);
    setError(null);
    const mode: StartMode = force ? 'force' : 'check';
    const body: StartTicket = {
      agent,
      ...(noSession ? { noSession: true } : { sessionMode }),
      ...(!personal && branch.trim() ? { branch: branch.trim() } : {}),
      ...(!personal && base.trim() ? { base: base.trim() } : {}),
      ...(noSession ? {} : { permissionMode }),
      ...(newWorkspace ? { newWorkspace: true } : placement ? { tab: placement } : {}),
      mode,
      ...INITIAL_SIZE,
    };
    try {
      const r = await api.startTicket(t.id, body);
      onStarted(r);
      onClose();
    } catch (e) {
      if (e instanceof ApiError && e.code === 'start_blocked') {
        setServerBlocked(e.message);
      } else {
        setError(actionError(e));
        requestAnimationFrame(() => errorRef.current?.focus());
      }
    } finally {
      setPending(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title={t ? `Start ${t.key}` : 'Start ticket'}
      description={t?.title}
      size="md"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>Cancel</Button>
          {force ? (
            <>
              <Button variant={conflict ? 'danger' : 'secondary'} pending={pending && !offerWait} disabled={!startable} onClick={() => void submit()}>
                Start anyway
              </Button>
              {offerWait && (
                <Button type="submit" form="start-ticket" variant="primary" pending={pending} disabled={!startable}>Wait</Button>
              )}
            </>
          ) : (
            <Button type="submit" form="start-ticket" variant="primary" pending={pending} disabled={!startable}>Start</Button>
          )}
        </>
      }
    >
      {!t ? (
        q.isPending ? <Skeleton lines={3} label="Loading ticket" /> : <Alert tone="error">{q.error?.message ?? 'Ticket not found.'}</Alert>
      ) : (
        <form id="start-ticket" className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); primary(); }}>
          {active && <Alert tone="info">{t.key} already has an active attempt{t.attempt!.branch ? ` on ${t.attempt!.branch}` : ''}.</Alert>}
          {closed && <Alert tone="info">{t.key} is closed. Move it back to Ready to start it.</Alert>}
          {blockers.length > 0 && (
            <Alert tone="warning" title={`Blocked by ${blockers.length === 1 ? 'an open ticket' : `${blockers.length} open tickets`}`}>
              <ul className="mt-1 flex flex-col gap-1">
                {blockers.map((b) => (
                  <li key={b.id} className="flex min-w-0 items-center gap-2">
                    <TicketKey ticketKey={b.key} />
                    <span className="min-w-0 truncate">{b.title}</span>
                  </li>
                ))}
              </ul>
            </Alert>
          )}
          {overlaps.map((o) => (
            <Alert
              key={o.ticketId}
              tone={o.level === 'conflict' ? 'error' : 'warning'}
              title={<span className="flex min-w-0 items-center gap-1.5">{o.level === 'conflict' ? 'Conflicts with' : 'Overlaps'} <TicketKey ticketKey={o.key} /></span>}
            >
              <div className="flex min-w-0 flex-col gap-1">
                <span className="min-w-0 truncate">{o.title}</span>
                {o.areas.length > 0 && (
                  <span className="flex min-w-0 flex-wrap items-center gap-1">Shared areas: <AreaChips areas={o.areas} config={config} /></span>
                )}
                {o.files.length > 0 && (
                  <>
                    <span>Shared files:</span>
                    <SharedFiles files={o.files} total={o.filesTotal} />
                  </>
                )}
              </div>
            </Alert>
          ))}
          {serverBlocked && blockers.length === 0 && overlaps.length === 0 && <Alert tone="warning">{serverBlocked}</Alert>}
          {waiting && startable && (
            <Alert
              tone="info"
              title="Waiting until clear"
              action={<Button size="sm" variant="secondary" disabled={pending} onClick={() => void stopWaiting()}>Stop waiting</Button>}
            >
              {t.clearToStart ? 'Nothing is in the way any more.' : `${t.key} waits for its blockers and overlaps to clear.`}
            </Alert>
          )}
          <Field
            label="Session"
            hint={
              noSession
                ? personal
                  ? "No agent yet: just the ticket's folder and the ticket in its workspace."
                  : 'No agent yet: just the branch, worktree and the ticket in its workspace.'
                : sessionMode === 'conversation' ? 'The ticket is its first message.' : "The agent's own terminal, with the ticket as its first prompt."
            }
          >
            <SegmentedControl<SessionMode | 'none'>
              aria-label="Session"
              options={[{ value: 'conversation', label: 'Conversation' }, { value: 'terminal', label: 'Terminal', disabled: isAcp(agent) }, { value: 'none', label: 'No session' }]}
              value={view}
              onChange={setMode}
            />
          </Field>
          {!noSession && (
            <Field label="Agent">
              {/* A Select, not a segmented control: the agents (with ACP ones and login hints) outgrow the dialog's width (PDT-470). */}
              <Select<AgentKind>
                options={offeredAgents.map((a) => ({ value: a, label: availability.hint(a) ? `${agents.label(a)} · ${availability.hint(a)}` : agents.label(a) }))}
                value={agent}
                onChange={setAgent}
                className="w-64"
              />
            </Field>
          )}
          {personal ? (
            <p className="text-xs text-muted-foreground">
              {project.name} is a Personal project (no repository): the work happens in the ticket's own folder, with no branch or worktree.
            </p>
          ) : (
            <>
              <Field label="Branch" hint="Made from the base with a worktree; an existing .worktrees folder for it is reused.">
                <Input mono value={branch} placeholder={t.defaultBranch} maxLength={200} onChange={(e) => setBranch(e.target.value)} />
              </Field>
              <Field label="Base branch">
                <Input mono value={base} placeholder={project?.baseBranch ?? 'main'} maxLength={200} onChange={(e) => setBase(e.target.value)} />
              </Field>
            </>
          )}
          {!noSession && sessionMode === 'conversation' && (
            <Field label="Permissions">
              <PermissionModeSelect value={permissionMode} onChange={setPermission} size="md" className="w-48" />
            </Field>
          )}
          {!noSession && sessionMode === 'terminal' && (
            <Checkbox checked={permissionMode === 'bypass'} onChange={(on) => setPermission(on ? 'bypass' : 'ask')}>
              Skip permission prompts (dangerous)
            </Checkbox>
          )}
          <Switch size="sm" checked={newWorkspace} onChange={setNewWorkspace} className="text-xs">
            Open in a new workspace
          </Switch>
          {error && <div ref={errorRef} tabIndex={-1} className="outline-none"><Alert tone="error">{error}</Alert></div>}
        </form>
      )}
    </Dialog>
  );
}
