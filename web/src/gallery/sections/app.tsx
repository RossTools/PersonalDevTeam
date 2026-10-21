import { useState, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Button } from '@ui/Button';
import { Toaster } from '@ui/Toaster';
import { ApiError } from '../../api/client';
import type { Project } from '../../api/gen/Project';
import type { Worktree } from '../../api/gen/Worktree';
import { AgentDot, AgentLabel, agentState, rollup, type AgentStateInput } from '../../common/AgentState';
import { BranchChip, ProjectKey } from '../../common/BranchChip';
import { Composer } from '../../common/Composer';
import { InlineRename } from '../../common/InlineRename';
import { PlacePicker, type Place } from '../../common/PlacePicker';
import { useAction } from '../../common/useAction';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}

const states: [string, AgentStateInput][] = [
  ['Claude A', { kind: 'agent_tui', status: 'running', activity: 'working' }],
  ['Codex', { kind: 'agent_tui', status: 'running', activity: 'needs_you', unseen: true }],
  ['Reviewer', { kind: 'agent_tui', status: 'running', activity: 'idle', unseen: true }],
  ['Builder', { kind: 'agent_tui', status: 'running', activity: 'idle' }],
  ['Old agent', { kind: 'agent_tui', status: 'finished' }],
  ['After restart', { kind: 'agent_tui', status: 'interrupted' }],
  ['zsh', { kind: 'shell', status: 'running' }],
  ['Test · main', { kind: 'run', status: 'failed' }],
  ['pdt · phase-3c', { kind: 'run', status: 'starting' }],
];

const projects: Project[] = [
  { id: 'p1', name: 'PersonalDevTeam', key: 'PDT', repoPath: '/work/pdt', baseBranch: 'main', createdAt: 0, githubRepo: 'ross/pdt', kind: 'repo' },
  { id: 'p2', name: 'Demo', key: 'DEMO', repoPath: '/work/demo', baseBranch: 'trunk', createdAt: 0, githubRepo: null, kind: 'repo' },
];
const wt = (id: string, projectId: string, branch: string): Worktree =>
  ({ id, projectId, branch, path: `/work/${branch}`, baseCommit: 'abc', createdAt: 0, missing: false });

/** A query cache seeded with static worktrees, so PlacePicker needs no daemon. */
const seeded = (() => {
  const qc = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
  qc.setQueryData(['worktrees', 'p1'], [wt('w1', 'p1', 'phase-3c-1-ui'), wt('w2', 'p1', 'fix/terminal-links')]);
  qc.setQueryData(['worktrees', 'p2'], [wt('w3', 'p2', 'feature')]);
  return qc;
})();

function PlaceDemo() {
  const [place, setPlace] = useState<Place>({ projectId: 'p1', worktreeId: 'w1' });
  return (
    <QueryClientProvider client={seeded}>
      <PlacePicker projects={projects} place={place} onPick={setPlace} />
    </QueryClientProvider>
  );
}

function RenameDemo() {
  const [name, setName] = useState('Claude A');
  const [editing, setEditing] = useState(false);
  return editing ? (
    <InlineRename label="Tab name" initial={name} onCommit={(v) => { setName(v); setEditing(false); }} onCancel={() => setEditing(false)} className="w-40" />
  ) : (
    <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>{name} (click to rename)</Button>
  );
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function ActionDemo() {
  const [closeOk, closingOk] = useAction(() => wait(800), { success: 'Tab closed' });
  const [closeBad, closingBad] = useAction(async () => {
    await wait(800);
    throw new ApiError(409, 'session_busy', 'The session is still starting; try again in a moment.');
  });
  return (
    <>
      <Toaster />
      <Button pending={closingOk} onClick={() => void closeOk()}>Close tab (succeeds)</Button>
      <Button pending={closingBad} onClick={() => void closeBad()}>Close tab (fails)</Button>
    </>
  );
}

function render() {
  const values = states.map(([, s]) => agentState(s));
  const ws = rollup(values);
  return (
    <div className="flex flex-col gap-5">
      <Row label="AgentState: dot + label">
        {states.map(([title], i) => (
          <span key={title} className="inline-flex items-center gap-1.5 text-xs">
            <AgentDot state={values[i]} />
            {title}
            <AgentLabel state={values[i]} />
          </span>
        ))}
      </Row>
      <Row label="Workspace rollup">
        {ws && (
          <span className="inline-flex items-center gap-1.5 text-xs">
            <AgentDot state={ws} /> Workspace <AgentLabel state={ws} />
          </span>
        )}
      </Row>
      <Row label="BranchChip, ProjectKey">
        <BranchChip branch="phase-3c-1-ui" repo="ross/pdt" base="main" />
        <BranchChip branch="main" repo="ross/pdt" base="main" />
        <BranchChip branch="feature/local-only" />
        <ProjectKey projectKey="PDT" />
      </Row>
      <Row label="PlacePicker">
        <PlaceDemo />
      </Row>
      <Row label="InlineRename">
        <RenameDemo />
      </Row>
      <Row label="useAction">
        <ActionDemo />
      </Row>
      <Row label="Composer">
        <div className="w-full max-w-xl">
          <Composer
            autoFocus={false}
            onSend={async (text) => {
              await wait(600);
              if (text.includes('fail')) throw new Error('The session ended before the prompt was sent.');
            }}
            onCancel={() => {}}
            placeholder='Enter sends, Shift+Enter adds a line. Type "fail" to see the error.'
          />
        </div>
      </Row>
    </div>
  );
}

export const section = { title: 'App components', order: 50, render };
