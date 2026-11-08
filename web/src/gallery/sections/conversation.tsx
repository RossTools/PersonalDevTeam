import { useState, type ReactNode } from 'react';
import { ApprovalCard, type ApprovalAnswer } from '@ui/ApprovalCard';
import { Eye, Pencil } from 'lucide-react';
import { DiffViewer } from '@ui/DiffViewer';
import { PreviewBox } from '@ui/PreviewBox';
import { ToolCard } from '@ui/ToolCard';
import { ImageThumb } from '@ui/ImageThumb';
import { SuggestionList } from '@ui/SuggestionList';
import { ConversationComposer } from '../../conversation/ConversationComposer';
import { MessageBubble } from '@ui/MessageBubble';
import { PromptBox } from '@ui/PromptBox';
import { Select } from '@ui/Select';
import { ToolCallRow, ToolOutput } from '@ui/ToolCallRow';
import { toast } from '@ui/Toaster';
import { Markdown } from '../../common/Markdown';
import type { ConversationEvent } from '../../api/gen/ConversationEvent';
import { MessageList } from '../../conversation/MessageList';
import { SectionLabel } from '@ui/SectionLabel';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      {children}
    </div>
  );
}

function PromptBoxDemo({ working, phone }: { working?: boolean; phone?: boolean }) {
  const [text, setText] = useState(working && !phone ? 'Also update the changelog.' : '');
  const [model, setModel] = useState('default');
  return (
    <PromptBox
      label="Message"
      value={text}
      onChange={setText}
      onSend={() => setText('')}
      onStop={working ? () => toast.info('Stop') : undefined}
      sendLabel={working ? 'Queue' : 'Send'}
      placeholder="Message Claude…"
      layout={phone ? 'phone' : 'default'}
      toolbar={
        <>
          <Select aria-label="Model" size="sm" quiet value={model} onChange={setModel}
            options={[{ value: 'default', label: 'Default model' }, { value: 'opus', label: 'Opus' }]} />
          <Select aria-label="Permission mode" size="sm" quiet value="ask" onChange={() => {}} options={[{ value: 'ask', label: 'Ask' }]} />
        </>
      }
      meta={phone ? (working ? 'working' : 'ready') : working ? 'Esc stops' : 'Context 16% · 31k / 200k'}
    />
  );
}

const DIFF = [
  '--- a/tests/login.rs',
  '+++ b/tests/login.rs',
  '@@ -40,7 +40,9 @@ async fn login_flow() {',
  '     let app = spawn_app().await;',
  '     app.post_login("alex", "secret").await;',
  '-    tokio::time::sleep(Duration::from_millis(100)).await;',
  '-    assert!(app.has_session_cookie());',
  '+    // Poll instead of a fixed sleep: CI machines are slower.',
  '+    wait_until(|| app.has_session_cookie(), Duration::from_secs(5)).await;',
  '+    assert!(app.has_session_cookie(), "no session cookie after login");',
  '     app.get("/home").await.assert_ok();',
  ' }',
  '--- /dev/null',
  '+++ b/tests/support/wait.rs',
  '@@ -0,0 +1,3 @@',
  '+pub async fn wait_until(f: impl Fn() -> bool, max: Duration) {',
  '+    // …',
  '+}',
].join('\n');

const PICTURE = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 80"><rect width="80" height="80" fill="lavender"/><circle cx="28" cy="30" r="10" fill="khaki"/><path d="M0 70 L30 40 L50 58 L62 46 L80 64 L80 80 L0 80Z" fill="slateblue"/></svg>')}`;

const answered = (a: ApprovalAnswer) => void toast.info(`Answer: ${JSON.stringify(a)}`);
const T0 = Date.now() - 60_000;
const e = (seq: number, event: ConversationEvent['event'], turnId = 'u1'): ConversationEvent => ({ seq, pos: seq, ts: T0 + seq * 1000, itemId: `i${seq}`, turnId, event });

const sample: ConversationEvent[] = [
  e(1, { kind: 'user_message', text: 'The login test is flaky on CI. Can you look?', images: [], source: 'pdt' }),
  e(2, { kind: 'turn', phase: 'started' }),
  e(3, { kind: 'tool_call', id: 'a', tool: 'Grep', title: 'login_flow', input: { pattern: 'login_flow' }, status: 'done' }),
  e(4, { kind: 'tool_call', id: 'b', tool: 'Read', title: 'tests/login.rs', input: { file_path: 'tests/login.rs' }, status: 'done' }),
  e(5, { kind: 'tool_call', id: 'c', tool: 'Bash', title: 'cargo test login', input: { command: 'cargo test login' }, status: 'done', output: 'running 3 tests\ntest login_ok ... ok\ntest login_flow ... ok\n\ntest result: ok. 3 passed' }),
  e(6, { kind: 'tool_call', id: 'f', tool: 'Edit', title: 'tests/login.rs', input: { file_path: 'tests/login.rs' }, status: 'done', diff: DIFF.split('--- /dev/null')[0] }),
  e(7, { kind: 'agent_message', id: 'm1', phase: 'final', markdown: 'The test waits on a **fixed 100 ms sleep**. I replaced it with a poll on the session cookie.' }),
  e(8, { kind: 'turn_diff', diff: DIFF }),
  e(9, { kind: 'turn', phase: 'ended', status: 'completed', durationMs: 42_000 }),
  e(10, { kind: 'user_message', text: 'Great, run the whole suite.', images: [], source: 'pdt' }, 'u2'),
  e(11, { kind: 'turn', phase: 'started' }, 'u2'),
  e(12, { kind: 'tool_call', id: 'd', tool: 'Bash', title: 'cargo test', input: { command: 'cargo test' }, status: 'running' }, 'u2'),
];

const cutOff: ConversationEvent[] = [
  e(1, { kind: 'user_message', text: 'Upgrade the dependencies.', images: [], source: 'pdt' }),
  e(2, { kind: 'turn', phase: 'started' }),
  e(3, { kind: 'tool_call', id: 'a', tool: 'Bash', title: 'cargo update', input: { command: 'cargo update' }, status: 'done' }),
  e(4, { kind: 'turn', phase: 'ended', status: 'interrupted', reason: 'PDT restarted' }),
];

export const section = {
  title: 'Conversation',
  order: 75,
  render: () => (
    <div className="flex max-w-3xl flex-col gap-6">
      <Demo label="PromptBox · idle, working (Stop + Queue)">
        <div className="flex flex-col gap-3">
          <PromptBoxDemo />
          <PromptBoxDemo working />
        </div>
      </Demo>
      <Demo label="PromptBox · phone layout: idle, working with an empty box (⬆ is Stop)">
        <div className="flex max-w-sm flex-col gap-3">
          <PromptBoxDemo phone />
          <PromptBoxDemo phone working />
        </div>
      </Demo>
      <Demo label="Composer · / commands (TUI-only shown, not pickable), @ files, images, a rate-limit warning">
        <div className="flex flex-col gap-3">
          <div className="pt-36">
            <SuggestionList
              id="g-cmd"
              label="Commands"
              active={0}
              onPick={() => {}}
              items={[
                { id: 'compact', label: '/compact', detail: 'Compact the conversation' },
                { id: 'context', label: '/context', detail: 'Show what fills the context' },
                { id: 'config', label: '/config', disabled: true, hint: 'Only in the terminal' },
              ]}
            />
          </div>
          <SuggestionList id="g-files" label="Files" active={1} onPick={() => {}} items={[{ id: 'a', label: 'src/main.rs' }, { id: 'b', label: 'web/src/main.tsx' }]} />
          <div className="flex gap-2">
            <ImageThumb src={PICTURE} alt="screenshot.png" size="sm" onRemove={() => {}} />
            <ImageThumb src={PICTURE} alt="uploading.png" size="sm" pending onRemove={() => {}} />
            <ImageThumb src={PICTURE} alt="broken.png" size="sm" error="Images can be at most 10 MB" onRemove={() => {}} />
            <ImageThumb src={PICTURE} alt="sent" href={PICTURE} />
            <ImageThumb alt="from the terminal" />
          </div>
          <ConversationComposer
            turnRunning={false}
            onSend={async () => {}}
            onInterrupt={() => {}}
            models={[]}
            model={null}
            onModel={() => {}}
            permissionModes={['ask', 'accept_edits', 'plan', 'bypass']}
            permissionMode="ask"
            onPermissionMode={() => {}}
            readout="Claude plan · Context 16% · 31k / 200k"
            limit={{ text: 'Weekly limit 86% · resets Tue 3:40 PM', reached: false }}
            commands={[{ name: 'compact', description: 'Compact the conversation' }, { name: 'review', description: 'Review a pull request' }]}
            terminalCommands={['config', 'login']}
            onSearchFiles={async (q) => ['src/main.rs', 'web/src/main.tsx', 'README.md'].filter((p) => p.includes(q))}
            onUpload={async () => ({ id: 'gallery' })}
          />
        </div>
      </Demo>
      <Demo label="MessageBubble · sent, queued, from the terminal, with an image">
        <div className="flex flex-col gap-2">
          <MessageBubble images={<ImageThumb src={PICTURE} href={PICTURE} alt="Image 1" />}>Why does the header wrap here?</MessageBubble>
          <MessageBubble>Fix the flaky login test, then open a PR.</MessageBubble>
          <MessageBubble tone="pending" label="Queued">Also update the changelog.</MessageBubble>
          <MessageBubble meta="From the terminal">/compact</MessageBubble>
        </div>
      </Demo>
      <Demo label="ToolCallRow · running, done, failed, declined, nested, summary">
        <div className="flex flex-col">
          <ToolCallRow verb="Run" subject="cargo test --workspace" status="running" meta="0:42" />
          <ToolCallRow verb="Edit" subject="crates/pdt-api/src/start.rs" status="done" meta="+12 −3"
            details={<ToolOutput label="Diff">{'-    sleep(100);\n+    wait_for_cookie().await?;'}</ToolOutput>} />
          <ToolCallRow verb="Run" subject="pnpm test" status="failed" meta="exit 1"
            details={<ToolOutput label="Output" tone="error">{'\u001b[31mFAIL\u001b[0m src/login.test.ts\n  expected 200, got 401'}</ToolOutput>} />
          <ToolCallRow verb="Write" subject="/etc/hosts" status="declined" />
          <ToolCallRow verb="Agent:" subject="Explore the auth module" status="done" defaultOpen>
            <ToolCallRow verb="Search" subject="fn login" status="done" />
            <ToolCallRow verb="Read" subject="src/auth.rs" status="done" />
          </ToolCallRow>
          <ToolCallRow verb="Ran 3 commands, edited 2 files">
            <ToolCallRow verb="Run" subject="ls" status="done" />
          </ToolCallRow>
        </div>
      </Demo>
      <Demo label="ApprovalCard · command (with Always), file, keyboard y / n">
        <div className="flex flex-col gap-3">
          <ApprovalCard title="Run this command?" subject="cargo test -p pdt-api" description="Run the API crate's tests" canAlways keyboard onAnswer={answered} />
          <ApprovalCard title="Edit this file?" preview={<DiffViewer diff={DIFF.split('--- /dev/null')[0]} />} onAnswer={answered} />
        </div>
      </Demo>
      <Demo label="ToolCard · a heading with an aside, rows; PreviewBox · what a request carries">
        <div className="flex flex-col gap-3">
          <ToolCard icon={<Pencil />} aria-label="Changed SHOP-19" heading={<>Changed <span className="font-medium text-foreground">SHOP-19</span></>} aside={<span>Queued</span>}>
            <div className="py-2">Stage: To do → In progress</div>
            <div className="py-2">Priority: Low → High</div>
          </ToolCard>
          <ToolCard icon={<Eye />} aria-label="Stopped watching" heading="Stopped watching" />
          <PreviewBox className="flex flex-col gap-1.5">
            <span className="font-semibold">Guest checkout: email-only orders</span>
            <span className="text-xs text-muted-foreground">In SHOP · To do · High</span>
          </PreviewBox>
        </div>
      </Demo>
      <Demo label="DiffViewer · two files (one new); hunks without line numbers">
        <div className="flex flex-col gap-3">
          <DiffViewer diff={DIFF} />
          <DiffViewer diff={'--- a/notes.md\n+++ b/notes.md\n@@ @@\n-line two\n+pear'} />
        </div>
      </Demo>
      <Demo label="ApprovalCard · questions (descriptions, several picks, Something else), plan (Claude: accept edits)">
        <div className="flex flex-col gap-3">
          <ApprovalCard
            kind="question"
            title="Claude asks"
            questions={[
              {
                question: 'Which database should the cache use?',
                header: 'Storage',
                options: [{ label: 'SQLite', description: 'Survives restarts' }, { label: 'In memory', description: 'Fastest; lost on restart' }],
              },
              { question: 'What should it cache?', options: [{ label: 'Tickets' }, { label: 'Sessions' }, { label: 'Runs' }], multiSelect: true },
            ]}
            onAnswer={answered}
          />
          <ApprovalCard kind="plan" title="Approve this plan?" planAcceptEdits plan={<Markdown>{'1. Add `session_events`\n2. Stream deltas over the socket\n3. Render the conversation'}</Markdown>} onAnswer={answered} />
        </div>
      </Demo>
      <Demo label="MessageList · a finished turn (collapsed) and a running one">
        <div className="flex h-96 flex-col rounded-lg border border-border">
          <MessageList rows={sample} provisional={[]} streams={{}} turnRunning pending={[{ id: 'p', text: 'Then push it.', delivery: 'next', images: [], notice: false }]} onAnswer={async () => {}} />
        </div>
      </Demo>
      <Demo label="MessageList · a turn cut off by a restart (Continue)">
        <div className="flex h-48 flex-col rounded-lg border border-border">
          <MessageList rows={cutOff} provisional={[]} streams={{}} turnRunning={false} pending={[]} onAnswer={async () => {}} onContinue={async () => void toast.info('Continue')} />
        </div>
      </Demo>
    </div>
  ),
};
