import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, ChevronDown, Minus } from 'lucide-react';
import { useId, useState, type FormEvent, type ReactNode } from 'react';
import { Alert, Badge, Button, ChoiceCardGroup, Input, Link, Select, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { HostChecks } from '../api/gen/HostChecks';
import type { SetupView } from '../api/gen/SetupView';
import type { StepState } from '../api/gen/StepState';
import type { AddProjectMode } from '../projects/AddProjectForm';
import { useAction } from '../common/useAction';
import type { AcpAgentView } from '../api/gen/AcpAgentView';
import { agentLabel, isAcp, useAcpAgents, useAgentsOff, type BuiltinAgent } from '../common/agents';
import { checkOf } from '../common/useAgentAvailability';
import { useProjects } from '../projects/queries';
import { hostKeys, useHostSettings, setHostSettings, useHostChecks } from '../host/queries';
import { POLL } from '../common/poll';

export type StepId = 'agents' | 'github' | 'project' | 'defaultAgent';

const AGENTS: { id: BuiltinAgent; name: string; install: string; env: string }[] = [
  { id: 'claude', name: 'Claude Code', install: 'https://code.claude.com/docs/en/setup', env: 'PDT_CLAUDE_PROGRAM' },
  { id: 'codex', name: 'Codex', install: 'https://developers.openai.com/codex/cli', env: 'PDT_CODEX_PROGRAM' },
  { id: 'devin', name: 'Devin', install: 'https://cli.devin.ai', env: 'PDT_DEVIN_PROGRAM' },
];
/** Each agent's program in the host settings (Settings › This host › Agents › Programs). */
const PROGRAM = { claude: 'claudeProgram', codex: 'codexProgram', devin: 'devinProgram' } as const;
const GIT_INSTALL = 'https://git-scm.com/downloads';
const agentName = (a: AgentKind, acp?: AcpAgentView[]) => AGENTS.find((x) => x.id === a)?.name ?? agentLabel(a, acp);

/** Pure: the step to open first: the first one not done, in page order. */
export function firstOpenStep(v: SetupView): StepId | null {
  return (['agents', 'github', 'project', 'defaultAgent'] as const).find((k) => v.steps[k] === 'todo') ?? null;
}

/** Pure: the agents the checks found, in page order, less the ones switched off (PDT-270). */
export function foundAgents(checks: HostChecks | undefined, off: readonly AgentKind[] = []): BuiltinAgent[] {
  return AGENTS.map((a) => a.id).filter((a) => checks?.[a].found && !off.includes(a));
}

export interface SetupPageProps {
  /** Opens the Add project dialog (`clone`: on Clone URL). */
  onAddProject: (mode?: AddProjectMode) => void;
  onBoard: () => void;
  onPm: () => void;
  /** Settings › This host, at a section (by its slug). */
  onHostSettings: (section: string) => void;
  /** After Dismiss setup. */
  onDismissed: () => void;
}

/**
 * Get started (PDT-230): four steps PDT needs before agents can work on the user's code, in any order. Each ticks
 * itself off from the host's real state (the checks, the GitHub login, the projects, the default agent).
 */
export function SetupPage(props: SetupPageProps) {
  const qc = useQueryClient();
  // Polls, so steps done elsewhere (GitHub on a phone, a project added from the sidebar) tick off here.
  const setup = useQuery({ queryKey: hostKeys.setup, queryFn: api.hostSetup, refetchInterval: POLL.live });
  const checks = useHostChecks();
  const [open, setOpen] = useState<StepId | null | undefined>(undefined);
  const dismiss = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveHostSetup({ dismissed: true }),
    onSuccess: (v) => {
      qc.setQueryData(hostKeys.setup, v);
      props.onDismissed();
    },
  });
  const v = setup.data;
  if (!v) {
    return (
      <Shell>
        {setup.error ? <Alert tone="error">Get started could not be loaded: {setup.error.message}</Alert> : <Skeleton lines={6} label="Loading Get started" />}
      </Shell>
    );
  }
  // Until the user picks one, the first step not done is open.
  const shown = open === undefined ? firstOpenStep(v) : open;
  const toggle = (k: StepId) => setOpen(shown === k ? null : k);
  const step = (k: StepId) => ({ id: k, state: v.steps[k], open: shown === k, onToggle: () => toggle(k) });
  const next = (after: StepId) => {
    const order: StepId[] = ['agents', 'github', 'project', 'defaultAgent'];
    setOpen(order.slice(order.indexOf(after) + 1).find((k) => v.steps[k] === 'todo') ?? null);
  };
  return (
    <Shell>
      <div className="flex flex-col gap-1">
        <Logo />
        <h2 className="text-heading font-semibold">Get started</h2>
        <p className="text-muted-foreground">A few things PDT needs before agents can work on your code. Do them in any order. Each one ticks itself off.</p>
      </div>
      <Progress done={v.done} />
      {v.done === 4 && (
        <div role="status" className="flex flex-col gap-3 rounded-lg border border-running bg-running-soft p-4">
          <p><b>You’re set up.</b> Create a ticket and move it to Todo, or open the PM and ask it to plan something.</p>
          <div className="flex flex-wrap gap-2">
            <Button variant="primary" onClick={props.onBoard}>Go to the board</Button>
            <Button onClick={props.onPm}>Open the PM</Button>
          </div>
        </div>
      )}
      <div className="flex flex-col gap-2">
        <AgentsStep {...step('agents')} v={v} checks={checks.data} loading={checks.isPending} />
        <GithubStep {...step('github')} onDone={() => next('github')} />
        <ProjectStep {...step('project')} {...props} />
        <DefaultAgentStep {...step('defaultAgent')} v={v} checks={checks.data} />
      </div>
      <footer className="flex flex-wrap items-center justify-between gap-2 text-muted-foreground">
        <span>Reopen this any time from Settings › This host › Setup.</span>
        <Button variant="ghost" size="sm" pending={dismiss.isPending} onClick={() => dismiss.mutate()}>Dismiss setup</Button>
      </footer>
      {dismiss.error && <Alert tone="error">{dismiss.error.message}</Alert>}
    </Shell>
  );
}

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <article aria-label="Get started" className="mx-auto flex w-full max-w-2xl flex-col gap-4 p-4 text-sm sm:px-8 sm:py-10">
        {children}
      </article>
    </div>
  );
}

/** The pdt wordmark, underlined in the accent colour like the app icon. */
function Logo() {
  return (
    <div aria-hidden className="relative mb-2 inline-block w-fit font-mono text-title font-extrabold tracking-tighter">
      pdt
      <span className="absolute right-0 bottom-0.5 left-[0.62em] h-[3px] bg-primary" />
    </div>
  );
}

function Progress({ done }: { done: number }) {
  return (
    <div className="flex items-center gap-3 text-muted-foreground">
      <div role="progressbar" aria-label="Setup progress" aria-valuemin={0} aria-valuemax={4} aria-valuenow={done} className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
        <div className="h-full bg-running transition-[width] motion-reduce:transition-none" style={{ width: `${(done / 4) * 100}%` }} />
      </div>
      <span>{done} of 4</span>
    </div>
  );
}

interface StepProps {
  id: StepId;
  state: StepState;
  open: boolean;
  onToggle: () => void;
}

function Step({ id, state, open, onToggle, title, summary, tag, children }: StepProps & { title: string; summary: ReactNode; tag?: string; children: ReactNode }) {
  const bodyId = useId();
  const label = state === 'done' ? 'done' : state === 'skipped' ? 'skipped' : 'to do';
  return (
    <section aria-label={title} data-step={id} className={cn('overflow-hidden rounded-lg border border-border bg-card', open && 'border-primary')}>
      <button type="button" aria-expanded={open} aria-controls={bodyId} onClick={onToggle}
        className="flex w-full items-center gap-3 px-3.5 py-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <span role="img" aria-label={label} className={cn(
          'grid size-5.5 shrink-0 place-items-center rounded-full border-2 border-border',
          state === 'done' && 'border-running bg-running text-background',
          state === 'skipped' && 'border-dashed text-muted-foreground',
        )}>
          {state === 'done' ? <Check className="size-3.5" strokeWidth={3} /> : state === 'skipped' ? <Minus className="size-3.5" /> : null}
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <b className="font-semibold">{title}</b>
          <span className="text-xs text-muted-foreground wrap-anywhere">{summary}</span>
        </span>
        {tag && <Badge>{tag}</Badge>}
        <ChevronDown aria-hidden className={cn('size-4 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none', open && 'rotate-180')} />
      </button>
      {open && <div id={bodyId} className="flex flex-col gap-3 px-3.5 pb-3.5 sm:pl-12">{children}</div>}
    </section>
  );
}

function AgentsStep({ v, checks, loading, ...step }: StepProps & { v: SetupView; checks: HostChecks | undefined; loading: boolean }) {
  const qc = useQueryClient();
  const [byHand, setByHand] = useState(false);
  const [check, checking] = useAction(async () => {
    qc.setQueryData(hostKeys.checks, await api.refreshChecks());
    await qc.invalidateQueries({ queryKey: hostKeys.setup });
  });
  const off = useAgentsOff();
  const found = foundAgents(checks, off);
  const summary = loading ? 'Looking for git and the agents…'
    : found.length > 0
      ? `Found ${found.map((a) => agentName(a)).join(' and ')}. PDT needs at least one, and git.`
      : 'No agent found yet. PDT needs at least one (Claude Code, Codex or Devin), and git.';
  // An agent switched off in Settings → Agents isn't one PDT needs (PDT-270).
  const rows: { id: 'git' | BuiltinAgent; name: string; install: string }[] = [{ id: 'git', name: 'git', install: GIT_INSTALL }, ...AGENTS.filter((a) => !off.includes(a.id))];
  return (
    <Step {...step} title="Agents on this machine" summary={checks && !checks.git.found ? 'git is missing. PDT needs it, and at least one agent.' : summary}>
      {loading ? <Skeleton lines={4} label="Loading checks" /> : (
        <table className="w-full border-collapse">
          <tbody>
            {rows.map((r) => {
              const c = checks?.[r.id];
              return (
                <tr key={r.id} className="border-t border-border first:border-t-0">
                  <td className="py-1.5 pr-3 font-medium whitespace-nowrap">{r.name}</td>
                  {/* `w-full max-w-0`: the path takes the room left over, and cuts in the middle when that isn't enough (PDT-465). */}
                  <td className="w-full max-w-0 truncate py-1.5 pr-2 font-mono text-xs text-muted-foreground">
                    {c?.found && c.path ? <MiddlePath path={c.path} /> : c?.found ? '' : 'not found'}
                  </td>
                  <td className="py-1.5 text-right whitespace-nowrap">
                    {c?.found ? <span className="text-running">Found</span> : (
                      <span className="text-destructive">Missing · <Link href={r.install} external>How to install</Link></span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" pending={checking} onClick={() => void check()}>Check again</Button>
        <Button size="sm" variant="ghost" aria-expanded={byHand} onClick={() => setByHand(!byHand)}>Set a path by hand…</Button>
      </div>
      {byHand && <ProgramPathForm onSaved={() => void check()} />}
      {step.state === 'todo' && !loading && (
        <p className="text-xs text-muted-foreground">Install git and one agent, log in to it once in a terminal, then Check again.</p>
      )}
    </Step>
  );
}

/** A path on one line, cut in the middle when it doesn't fit: the folders shrink, the last part stays. The full path is in the tooltip. */
function MiddlePath({ path }: { path: string }) {
  // Either separator: on Windows the checks return native paths (`C:\…\npm\claude.cmd`).
  const cut = Math.max(path.lastIndexOf('/', path.length - 2), path.lastIndexOf('\\', path.length - 2));
  const head = cut > 0 ? path.slice(0, cut) : '';
  const tail = cut > 0 ? path.slice(cut) : path;
  return (
    <span className="flex min-w-0" title={path}>
      {head && <span className="truncate">{head}</span>}
      <span className={head ? 'shrink-0' : 'truncate'}>{tail}</span>
    </span>
  );
}

/** Saves an agent's program path in the host settings: for one installed where PDT doesn't look. */
function ProgramPathForm({ onSaved }: { onSaved: () => void }) {
  const qc = useQueryClient();
  const host = useHostSettings();
  const stored = (a: BuiltinAgent) => host.data?.settings[PROGRAM[a]] ?? '';
  const [agent, setAgent] = useState<BuiltinAgent>('claude');
  const [path, setPath] = useState<string | null>(null);
  const shown = path ?? stored(agent);
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveHostSettings({ [PROGRAM[agent]]: shown }),
    onSuccess: (s) => {
      setHostSettings(qc, s);
      onSaved();
    },
  });
  const fromEnv = !!host.data?.programs.find((p) => p.agent === agent)?.fromEnv;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    save.mutate();
  };
  return (
    <form onSubmit={submit} aria-label="Agent program path" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Select aria-label="Agent" className="w-36" options={AGENTS.map((a) => ({ value: a.id, label: a.name }))} value={agent}
          onChange={(a) => { setAgent(a); setPath(null); }} />
        <Input aria-label="Program path" mono className="min-w-48 flex-1" placeholder="/full/path/to/the/program" value={shown} disabled={fromEnv} onChange={(e) => setPath(e.target.value)} />
        <Button type="submit" size="sm" variant="primary" disabled={fromEnv} pending={save.isPending}>Save path</Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {fromEnv ? `${AGENTS.find((a) => a.id === agent)!.env} sets it on this host, and wins over a path saved here.`
          : 'A full path, or one starting with ~. Leave it empty to look on the search path again.'}
      </p>
      {save.error && <Alert tone="error">{save.error.message}</Alert>}
    </form>
  );
}

function GithubStep({ onDone, ...step }: StepProps & { onDone: () => void }) {
  const qc = useQueryClient();
  const login = useQuery({
    queryKey: ['forge-login', 'github'],
    queryFn: () => api.forgeLogin('github'),
    refetchInterval: (q) => (q.state.data?.pending ? POLL.live : false),
  });
  const device = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.forgeLoginDevice('github', {}),
    onSuccess: (v) => qc.setQueryData(['forge-login', 'github'], v),
  });
  const skip = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveHostSetup({ githubSkipped: true }),
    onSuccess: (v) => {
      qc.setQueryData(hostKeys.setup, v);
      onDone();
    },
  });
  const g = login.data;
  const account = g?.accounts[0];
  const summary = account ? `Connected as ${account.login}.`
    : step.state === 'skipped' ? 'Skipped: projects without GitHub merge locally.'
      : 'For pull requests, CI and issues. Skip it if your projects aren’t on GitHub.';
  return (
    <Step {...step} title="Connect GitHub" summary={summary} tag="Optional">
      {account ? (
        <p className="text-muted-foreground">PDT signs in to GitHub as <b className="text-foreground">{account.login}</b>.</p>
      ) : (
        <>
          <p className="text-xs text-muted-foreground">PDT shows a code; you enter it at github.com/login/device (a phone works too).</p>
          {g?.pending && (
            <div className="rounded-md border border-border bg-background px-2.5 py-2 font-mono text-xs">
              Code: <b className="text-sm">{g.pending.userCode}</b> · <Link href={g.pending.verificationUri} external>Open {g.pending.verificationUri.replace(/^https?:\/\//, '')}</Link>
            </div>
          )}
          {g?.error && <Alert tone="error" title="The login failed.">{g.error}</Alert>}
          <div className="flex flex-wrap items-center gap-2">
            {g?.deviceFlow && !g.pending && (
              <Button size="sm" variant="primary" pending={device.isPending} onClick={() => device.mutate()}>Connect GitHub</Button>
            )}
            {g && !g.deviceFlow && <span className="text-xs text-muted-foreground">Paste a token in Settings › Plugins › GitHub.</span>}
            {step.state !== 'skipped' && (
              <Button size="sm" pending={skip.isPending} onClick={() => skip.mutate()}>Skip, my projects aren’t on GitHub</Button>
            )}
          </div>
          {(device.error ?? skip.error) && <Alert tone="error">{(device.error ?? skip.error)!.message}</Alert>}
        </>
      )}
    </Step>
  );
}

function ProjectStep({ onAddProject, onHostSettings, ...step }: StepProps & SetupPageProps) {
  const projects = useProjects();
  const host = useHostSettings();
  const github = useQuery({ queryKey: ['forge-login', 'github'], queryFn: () => api.forgeLogin('github') });
  const first = projects.data?.[0];
  const summary = first ? `${first.name}${first.repoPath ? ` · ${first.repoPath}` : ''}` : 'A folder with a git repo. PDT makes a worktree for each ticket.';
  return (
    <Step {...step} title="Add a project" summary={summary}>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant={first ? 'secondary' : 'primary'} onClick={() => onAddProject()}>{first ? 'Add another…' : 'Choose a folder…'}</Button>
        {(github.data?.accounts.length ?? 0) > 0 && <Button size="sm" onClick={() => onAddProject('clone')}>Clone from GitHub…</Button>}
      </div>
      <p className="text-xs text-muted-foreground">
        Projects folder: <span className="font-mono">{host.data?.projectsDir ?? '~/work'}</span> · <Link onClick={() => onHostSettings('projects-folder')}>change</Link>
      </p>
    </Step>
  );
}

function DefaultAgentStep({ v, checks, ...step }: StepProps & { v: SetupView; checks: HostChecks | undefined }) {
  const qc = useQueryClient();
  const off = useAgentsOff();
  const found = foundAgents(checks, off);
  // The preset one, unless it isn't installed and another is.
  const acp = useAcpAgents();
  // An ACP agent the user made the default stays it.
  const preset = found.length > 0 && !isAcp(v.defaultAgent) && !found.some((a) => a === v.defaultAgent) ? found[0] : v.defaultAgent;
  const [picked, setPicked] = useState<AgentKind | null>(null);
  const agent = picked ?? preset;
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.saveHostSettings({ defaultAgent: agent }),
    onSuccess: (s) => {
      setHostSettings(qc, s);
      void qc.invalidateQueries({ queryKey: hostKeys.setup });
    },
  });
  const summary = step.state === 'done' ? `${agentName(v.defaultAgent, acp)} works on tickets unless a persona says otherwise.`
    : 'Which agent works on tickets by default. You can change it per persona later.';
  return (
    <Step {...step} title="Pick a default agent" summary={summary}>
      <ChoiceCardGroup
        aria-label="Default agent"
        size="sm"
        value={agent}
        onChange={(a) => setPicked(a)}
        options={AGENTS.filter((a) => !off.includes(a.id)).map((a) => {
          const there = !checks || checks[a.id].found;
          return { value: a.id, title: a.name, description: there ? 'Found' : 'Not installed', disabled: !there };
        })}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="primary" pending={save.isPending} disabled={checks !== undefined && !checkOf(checks, agent)?.found} onClick={() => save.mutate()}>
          Use {agentName(agent, acp)}
        </Button>
      </div>
      {save.error && <Alert tone="error">{save.error.message}</Alert>}
    </Step>
  );
}
