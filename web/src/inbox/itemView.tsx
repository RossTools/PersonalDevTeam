// What an Inbox item shows and offers (PDT-361), whatever the screen: one line on why it waits, its content in the
// kind's style, the note box and the actions. The desktop and the phone lay the same spec out their own way.
import { Fragment, useState, type ReactNode } from 'react';
import {
  AgentMessage, answerText, ApprovalCard, Callout, Checkbox, CodeCard, DiffViewer, EmptyState, Facts, Field, Input, NO_ANSWER, otherInputId, PassList, QuestionCard, Select, Textarea, type ApprovalAnswer, type QuestionAnswer,
} from '@ui';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { DecideRow } from '../api/gen/DecideRow';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Priority } from '../api/gen/Priority';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { LabelsEditor } from '../tickets/LabelsEditor';
import { LabelChips } from '../tickets/TicketParts';
import { TicketPicker } from '../tickets/TicketPicker';
import { approvalTitle, questionsOf as requestQuestions } from '../conversation/tools';
import { navigate } from '../shell/router';
import { useOpenSession, useOpenTicket } from '../common/open';
import { STORAGE_PATH, useCleanFinished } from '../projects/worktreeStorage';
import type { ItemLink } from './ItemLinks';
import {
  confirmOf, CONNECT_PATH, decideRowsOf, dismissable, gateQuestion, MCP_SERVERS_PATH, noticeText, outcomeTone, passesOf, proposalOf,
  inboxTargets, longTitle, proposalPath, questionsOf, resends, ticketProposalOf, triageOf,
} from './model';
import { answerApproval } from './approval';
import { allowRowsTitle, DecideRows, useKeptRows } from '../common/DecideRows';
import { nonBlank } from '../common/guards';
import { capitalize, formatBytes } from '../common/text';

/** One of an item's actions. */
export interface ItemAction {
  id: string;
  title: string;
  /** The phone's shorter label ("Send" for Send answers). */
  short?: string;
  primary?: boolean;
  /** Pushed to the right, away from the rest: Dismiss, Stop, Won't do, Deny. */
  end?: boolean;
  danger?: boolean;
  /** Its key, shown on the desktop: `↵` for the primary one unless it has its own. */
  kbd?: string;
  disabled?: boolean;
  pending?: boolean;
  /** A toggle (Take over): whether what it opens is open. */
  expanded?: boolean;
  run: () => void;
}

export interface ItemSpec {
  /** The content's name (a region). */
  region: string;
  /** One line: why the item waits on the user. */
  why: ReactNode;
  content: ReactNode;
  /** The note box, when the item takes a note. */
  note?: { label: string; placeholder: string; hint?: string };
  actions: ItemAction[];
  /** A line beside the actions (how many questions are answered). */
  hint?: ReactNode;
  /** Take over's outcomes, while it is open. */
  takeOver?: { outcomes: { id: string; title: string }[]; open: boolean; setOpen: (open: boolean) => void; pick: (id: string) => void };
  /** Links the kind adds to the links row. */
  links?: ItemLink[];
  /** A key the kind takes (number keys, `y`/`n`): true when it did something. */
  onKey?: (key: string) => boolean;
  /** Whether Enter takes the primary action (not an approval's Allow: `y` does). */
  enter: boolean;
}


/** Choices that stop, refuse or set aside go to the right, away from the primary one. */
function endChoice(id: string, title: string): boolean {
  return /^(stop|reject|deny|discard|close|ignore|wont_do|won't_do|cancel)$/i.test(id) || /^won.?t do$/i.test(title.trim());
}

/** The item's choices as actions, picked with `pick`. */
function choiceActions(item: InboxItem, pick: (id: string) => void, busy: boolean): ItemAction[] {
  return item.choices.map((c) => ({
    id: c.id,
    title: c.title,
    short: capitalize(c.title),
    primary: c.primary,
    end: !c.primary && endChoice(c.id, c.title),
    pending: busy && c.primary,
    disabled: busy,
    run: () => pick(c.id),
  }));
}

/** A low disk's figures, where the item has them: what is free. */
function diskFacts(item: InboxItem) {
  const free = typeof item.body.freeBytes === 'number' ? item.body.freeBytes : null;
  return free !== null ? [{ value: formatBytes(free), label: 'free on the disk' }] : [];
}

/**
 * The spec for `item`, with its own state (the note, the answers, Take over): give each item its own instance (key it).
 * `answer` and `dismiss` act on it; `onApproved` runs once an approval went through in its session.
 */
export function useItemView({ item, busy, answer, dismiss, onApproved, phone = false }: {
  item: InboxItem;
  busy: boolean;
  answer: (a: InboxAnswer) => Promise<void>;
  dismiss: () => void;
  onApproved: () => void;
  phone?: boolean;
}): { spec: ItemSpec; note: string; setNote: (s: string) => void } {
  const [note, setNote] = useState('');
  // A question's recommended choice starts picked (PDT-476); the user still sends it.
  const [answers, setAnswers] = useState<QuestionAnswer[]>(() =>
    item.kind === 'question' ? questionsOf(item).map((q) => (q.recommended ? { choice: q.recommended, other: null } : NO_ANSWER)) : []);
  const [active, setActive] = useState(0);
  const [takingOver, setTakingOver] = useState(false);
  const [secret, setSecret] = useState('');
  const [approving, setApproving] = useState(false);
  const [clean, cleaning] = useCleanFinished();
  // A ticket proposal's edits, the agentic box and Duplicate of… (PDT-366).
  const [proposed] = useState(() => ticketProposalOf(item));
  const [editing, setEditing] = useState(false);
  const [duplicating, setDuplicating] = useState(false);
  const [agentic, setAgentic] = useState(false);
  const [draft, setDraft] = useState(() => ({ title: item.title, description: proposed.description, priority: proposed.priority, labels: proposed.labels }));
  // A batch call's rows (PDT-475): an Inbox confirmation's, or the PM conversation's card's (PDT-502).
  const rows = decideRowsOf(item);
  const ticked = useKeptRows(rows?.length ?? 0);
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  const targets = inboxTargets(item);
  const withNote = (choice: string) => void answer({ choice, ...(note.trim() ? { text: note } : {}) });
  const plain = (choice: string) => void answer({ choice });
  const dismissAction: ItemAction[] = dismissable(item) ? [{ id: 'dismiss', title: 'Dismiss', kbd: 'e', end: true, run: dismiss }] : [];
  const workflow = nonBlank(item.body.workflow);
  const persona = item.personaName ?? 'The agent';

  // The PM's batch of decisions, from an Inbox confirmation or its conversation's card: untick rows, then Allow or Deny.
  const decideSpec = (rows: DecideRow[], allow: (kept: number[]) => void, deny: () => void, pending: boolean, session?: string | null) => {
    const { kept, isKept, toggle } = ticked;
    return {
      region: 'Agent call',
      why: `${session ?? item.personaName ?? 'The PM'} proposes ${rows.length === 1 ? 'a decision' : `${rows.length} decisions`} on your Inbox. Untick any you don't want; Allow applies the rest as your own decisions, Deny changes nothing.`,
      content: <DecideRows rows={rows} kept={isKept} onToggle={toggle} disabled={pending} />,
      actions: [
        { id: 'allow', title: allowRowsTitle(kept.length, rows.length), short: 'Allow', primary: true,
          pending, disabled: pending || kept.length === 0, run: () => allow(kept) },
        { id: 'deny', title: 'Deny', end: true, disabled: pending, run: deny },
      ],
      hint: `${kept.length} of ${rows.length} ticked`,
    };
  };

  const spec = ((): Omit<ItemSpec, 'enter'> & { enter?: boolean } => {
    switch (item.kind) {
      case 'question': {
        const qs = questionsOf(item);
        const at = (n: number) => answers[n] ?? NO_ANSWER;
        const set = (n: number, a: QuestionAnswer) => setAnswers(qs.map((_, i) => (i === n ? a : at(i))));
        const answered = qs.filter((_, n) => answerText(at(n)).trim()).length;
        const pickKey = (key: string) => {
          const n = Math.min(active, qs.length - 1);
          const q = qs[n];
          const k = Number(key);
          if (!q || !Number.isInteger(k) || k < 1 || k > q.choices.length + 1 || q.choices.length === 0) return false;
          if (k <= q.choices.length) {
            set(n, { choice: q.choices[k - 1].label, other: null });
            if (n + 1 < qs.length) setActive(n + 1);
          } else {
            set(n, { choice: at(n).choice, other: at(n).other ?? '' });
            document.getElementById(otherInputId(`${item.id}-q${n}`))?.focus();
          }
          return true;
        };
        return {
          region: 'Questions',
          why: `${persona} waits for your answers${item.ticketKey ? ` on ${item.ticketKey}` : ''}; its step carries on with them.`,
          // The title shows a lone short question; the card repeats only a longer one (asked before PDT-476), which
          // the title clamps. Its context is the agent's message above it.
          content: qs.map((q, n) => (
            <Fragment key={n}>
              {q.context && (
                <AgentMessage aria-label={qs.length > 1 ? `Context of question ${n + 1}` : 'Context'} name={persona}
                  label={`${persona}'s context`}>
                  <Markdown>{q.context}</Markdown>
                </AgentMessage>
              )}
              <QuestionCard id={`${item.id}-q${n}`} index={n} total={qs.length} label={q.question} numbers={!phone}
                question={qs.length === 1 && q.question === item.title && !longTitle(item) ? undefined : <Markdown>{q.question}</Markdown>}
                choices={q.choices.map((c) => ({ label: c.label, description: c.description ? <Markdown className="text-muted-foreground">{c.description}</Markdown> : undefined }))}
                recommended={q.recommended} why={q.why}
                value={at(n)} disabled={busy} onChange={(a) => { set(n, a); setActive(n); }} />
            </Fragment>
          )),
          actions: [
            { id: 'send', title: 'Send answers', short: 'Send', primary: true, pending: busy, disabled: answered === 0,
              run: () => void answer({ answers: qs.map((_, n) => answerText(at(n))) }) },
            ...dismissAction,
          ],
          hint: `${answered} of ${qs.length} answered`,
          onKey: pickKey,
        };
      }
      case 'secret': {
        const name = nonBlank(item.body.name) ?? 'secret';
        const purpose = nonBlank(item.body.purpose);
        const ok = secret.length >= 6;
        const send = () => {
          if (!ok || busy) return;
          void answer({ secret }).then(() => setSecret(''));
        };
        return {
          region: 'Secret request',
          why: (
            <>
              {nonBlank(item.body.sessionTitle) ?? 'An agent'} asks for the secret <code>{name}</code>{purpose ? `: ${purpose}` : '.'} It is
              kept in this machine&apos;s keychain; the agent uses it by name and never gets it in the chat. Never paste it into the chat.
            </>
          ),
          content: (
            <form onSubmit={(e) => { e.preventDefault(); send(); }}>
              <Field label={name} hint="At least 6 characters. You can change who may use it in Settings → Secrets.">
                <Input aria-label={`Value of ${name}`} type="password" autoComplete="off" value={secret} onChange={(e) => setSecret(e.target.value)} />
              </Field>
            </form>
          ),
          actions: [{ id: 'store', title: 'Store secret', primary: true, pending: busy, disabled: !ok, run: send }, ...dismissAction],
        };
      }
      case 'approval': {
        const tool = nonBlank(item.body.tool) ?? 'tool';
        const diff = nonBlank(item.body.diff);
        const description = nonBlank(item.body.description);
        const questions = requestQuestions(item.body.input);
        const why = `${item.personaName ?? nonBlank(item.body.sessionTitle) ?? 'The agent'}'s permission mode asks before this. It waits here until you decide.`;
        if (questions.length > 0) {
          // The agent's own questions (AskUserQuestion): the card asks them and sends the answers.
          return {
            region: 'Approval', why, actions: [],
            content: <ApprovalCard kind="question" title={approvalTitle(tool)} questions={questions} keyboard={!phone}
              onAnswer={(a) => answerApproval(item, a, onApproved)} />,
          };
        }
        const send = (a: ApprovalAnswer, kept?: number[]) => {
          setApproving(true);
          void answerApproval(item, a, onApproved, kept).finally(() => setApproving(false));
        };
        const allow = () => send({ type: 'approve', always: false });
        const deny = () => send({ type: 'deny', ...(note.trim() ? { message: note.trim() } : {}) });
        if (rows) {
          // The PM's decisions, asked on its conversation's card (PDT-502): the same table, answered there too.
          return decideSpec(rows, (kept) => send({ type: 'approve', always: false }, kept), () => send({ type: 'deny' }), approving);
        }
        const where = [tool, nonBlank(item.body.sessionTitle)].filter(Boolean).join(' · ');
        return {
          region: 'Approval', why, enter: false,
          content: (
            <CodeCard aria-label={approvalTitle(tool)} title={where} aside={description} code={diff ? undefined : item.title}>
              {diff && <DiffViewer diff={diff} />}
            </CodeCard>
          ),
          note: { label: 'Reason (optional)', placeholder: 'Why not (optional): the agent gets it with Deny' },
          actions: [
            { id: 'allow', title: 'Allow once', short: 'Allow', primary: true, kbd: 'y', pending: approving, disabled: approving, run: allow },
            ...(item.choices.some((c) => c.id === 'allow_always')
              ? [{ id: 'always', title: 'Always allow', short: 'Always', disabled: approving, run: () => send({ type: 'approve', always: true }) }]
              : []),
            { id: 'deny', title: 'Deny', kbd: 'n', end: true, danger: true, disabled: approving, run: deny },
          ],
          onKey: (key) => {
            if (approving) return false;
            if (key === 'y') allow();
            else if (key === 'n') deny();
            else return false;
            return true;
          },
        };
      }
      case 'session': {
        const open = () => { if (targets.session) openSession(targets.session); };
        return {
          region: 'Session waiting',
          why: `${nonBlank(item.body.sessionTitle) ?? 'This session'} waits on a permission prompt or a question. Answer it in the session; looking at it takes it out of the Inbox.`,
          content: null,
          actions: [{ id: 'open', title: 'Open session', primary: true, disabled: !targets.session, run: open }],
        };
      }
      case 'stalled': {
        const outcomes = item.choices.filter((c) => c.id.startsWith('outcome:')).map((c) => ({ id: c.id, title: c.id.slice('outcome:'.length) }));
        const message = nonBlank(item.body.message);
        const nudge: ItemAction = { id: 'nudge', title: 'Nudge', primary: !resends(item), pending: busy && !resends(item), disabled: busy, run: () => withNote('nudge') };
        return {
          region: 'Stalled step',
          why: `${nonBlank(item.body.why) ?? 'The step stopped without an outcome.'} Nudge it to carry on, or take it over and pick the outcome yourself.`,
          content: message && (
            <AgentMessage name="PDT" label="The message it never got">
              <Markdown>{message}</Markdown>
            </AgentMessage>
          ),
          note: { label: 'Nudge message (optional)', placeholder: 'Your words for the nudge (optional)' },
          actions: [
            // Its first message never reached the session: sending it again comes first (PDT-144).
            ...(resends(item)
              ? [{ id: 'resend', title: 'Resume with this message', primary: true, pending: busy, disabled: busy, run: () => plain('resend') }]
              : []),
            nudge,
            { id: 'takeover', title: 'Take over', disabled: outcomes.length === 0, expanded: takingOver, run: () => setTakingOver((t) => !t) },
            ...choiceActions({ ...item, choices: item.choices.filter((c) => c.id === 'stop') }, plain, busy),
            ...dismissAction,
          ],
          takeOver: { outcomes, open: takingOver, setOpen: setTakingOver, pick: (id) => { setTakingOver(false); plain(id); } },
        };
      }
      case 'timed_out':
        return {
          region: 'Timed out',
          why: `${nonBlank(item.body.node) ? `Step ${item.body.node as string}` : 'The step'}${workflow ? ` of workflow ${workflow}` : ''} ended without an outcome: ${nonBlank(item.body.why) ?? 'it ran out of time'}. Resume runs it again on the same ticket and attempt${item.sessionId ? ', in the same session' : ''}; Stop leaves it stopped.`,
          content: null,
          note: { label: 'Note (optional)', placeholder: 'A note for the agent with Resume (optional)' },
          actions: choiceActions(item, (c) => (c === 'resume' ? withNote(c) : plain(c)), busy),
        };
      case 'loop_limit': {
        const passes = passesOf(item);
        return {
          region: 'Loop limit',
          why: `This step has run as many times as its loop limit allows${typeof item.body.limit === 'number' ? ` (${item.body.limit})` : ''}, so a disagreement doesn't run forever.`,
          content: passes.length > 0 && (
            <PassList aria-label="Its passes so far" passes={passes.map((p, n) => ({
              name: p.persona, title: `Pass ${n + 1} · ${p.persona}`, outcome: p.outcome, tone: outcomeTone(p.outcome),
              summary: p.summary,
            }))} />
          ),
          actions: choiceActions(item, plain, busy),
        };
      }
      case 'manual': {
        const prompt = nonBlank(item.body.prompt);
        return {
          region: 'Your turn',
          why: `This stage is yours${workflow ? ` in workflow ${workflow}` : ''}: do it, then pick how it ended.`,
          content: prompt && (
            <AgentMessage name={nonBlank(item.body.persona) ?? workflow ?? 'Workflow'} label="What the stage asks">
              <Markdown>{prompt}</Markdown>
            </AgentMessage>
          ),
          note: { label: 'What you did (optional)', placeholder: 'What you did or found (optional): the next step reads it' },
          actions: choiceActions(item, withNote, busy),
        };
      }
      case 'connect':
        return {
          region: 'Connect GitHub',
          why: `It goes away once GitHub is connected${item.body.reason === 'rejected' ? ' again' : ''}.`,
          content: <Callout tone="warning"><Markdown>{noticeText(item)}</Markdown></Callout>,
          actions: [{ id: 'settings', title: "Open GitHub's page", short: 'Open GitHub settings', primary: true, run: () => navigate(CONNECT_PATH) }],
        };
      case 'mcp_sign_in':
        return {
          region: 'Sign in again',
          why: 'It goes away once you sign in again, or sign out.',
          content: <Callout tone="warning"><Markdown>{noticeText(item)}</Markdown></Callout>,
          actions: [{ id: 'settings', title: 'Open MCP servers', primary: true, run: () => navigate(MCP_SERVERS_PATH) }],
        };
      case 'storage':
        return {
          region: "Can't save changes",
          why: 'It goes away on its own once PDT can save again.',
          content: <Callout tone="error"><Markdown>{noticeText(item)}</Markdown></Callout>,
          actions: [],
        };
      case 'low_disk': {
        const facts = diskFacts(item);
        return {
          region: 'Low disk space',
          why: 'Builds fail and PDT can’t save when the disk fills. It goes away on its own once there is more space.',
          content: (
            <>
              {facts.length > 0 && <Facts aria-label="Disk space" facts={facts} />}
              <Callout tone={item.body.critical === true ? 'error' : 'warning'}><Markdown>{noticeText(item)}</Markdown></Callout>
            </>
          ),
          actions: [
            { id: 'open', title: 'Open Storage', primary: true, run: () => navigate(STORAGE_PATH) },
            { id: 'clean', title: 'Clean all finished', pending: cleaning, disabled: cleaning, run: () => void clean() },
          ],
          links: [{ label: 'Storage settings', onClick: () => navigate(STORAGE_PATH) }],
        };
      }
      case 'notice':
        return {
          region: 'Unheard notice',
          why: item.attemptId
            ? `No agent session was open to tell${workflow ? ` (from ${workflow})` : ''}, so it waits for you. It goes away once the pull request is fixed, merged or closed.`
            : `${workflow ?? 'A workflow'} wants you to know. Act on it if you disagree, then dismiss it.`,
          content: <Callout tone="error"><Markdown>{noticeText(item)}</Markdown></Callout>,
          actions: [
            ...(targets.ticket
              ? [{ id: 'ticket', title: 'Open ticket', primary: true, run: () => openTicket(targets.ticket!) }]
              : []),
            ...dismissAction,
          ],
        };
      case 'admit': {
        const { summary, issue, external } = triageOf(item);
        return {
          region: 'Triaged ticket',
          why: `${workflow ?? 'Triage'} drafted this ticket. Approve moves it to To do${external ? ' and lets it in: it came from outside PDT, so only you can' : ''}; Reject moves it to Won't do. Approve doesn't label it agentic.`,
          content: (
            <>
              {issue && <div className="text-sm text-muted-foreground"><Markdown>{`From [${issue}](${issue})`}</Markdown></div>}
              {summary && <AgentMessage name={workflow ?? 'Triage'} label="Triage's notes"><Markdown>{summary}</Markdown></AgentMessage>}
            </>
          ),
          actions: choiceActions(item, plain, busy),
        };
      }
      case 'proposal': {
        const { workflow: name, version, note: said, summary } = proposalOf(item);
        return {
          region: 'Workflow proposal',
          why: `Accept makes version ${version} the current version of ${name || 'the workflow'}${item.body.enabled === true ? ', which runs from then on' : ''}; Discard sets it aside.`,
          content: (
            <>
              {said && <AgentMessage aria-label="Note" name={item.personaName ?? 'Agent'} label={`${item.personaName ?? 'The agent'}'s note`}><p className="whitespace-pre-wrap">{said}</p></AgentMessage>}
              {summary && <p aria-label="What changed" className="text-sm text-muted-foreground">{summary}</p>}
            </>
          ),
          actions: choiceActions(item, plain, busy),
          links: [{ label: 'Versions', 'aria-label': "Open the workflow's versions", onClick: () => navigate(proposalPath(item)) }],
        };
      }
      case 'ticket_proposal': {
        const by = proposed.persona ?? item.personaName ?? 'An agent';
        const edits = editing ? { title: draft.title, description: draft.description, priority: draft.priority, labels: draft.labels } : {};
        // Reject takes only the reason; Accept and Duplicate of… the edits and the box.
        const decide = (choice: string, extra: Record<string, unknown> = {}) =>
          void answer({ choice, ...(choice === 'reject' ? {} : { proposal: { ...edits, agentic, ...extra } }), ...(note.trim() ? { text: note } : {}) });
        const set = (patch: Partial<typeof draft>) => setDraft((d) => ({ ...d, ...patch }));
        return {
          region: 'Ticket proposal',
          why: `${by} proposed this ticket${proposed.fromTicket ? ` while working on ${proposed.fromTicket}` : ''}. It waits in Draft, off the board, until you decide; ${by} hears what you choose.`,
          content: (
            <>
              {editing ? (
                <div className="flex flex-col gap-3">
                  <Field label="Title"><Input value={draft.title} onChange={(e) => set({ title: e.target.value })} /></Field>
                  <Field label="Description"><Textarea rows={8} value={draft.description} onChange={(e) => set({ description: e.target.value })} /></Field>
                  <Field label="Priority">
                    <Select options={PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }))} value={draft.priority} onChange={(p: Priority) => set({ priority: p })} />
                  </Field>
                  <Field label="Labels"><LabelsEditor labels={draft.labels} onChange={(labels) => set({ labels })} /></Field>
                </div>
              ) : (
                <div aria-label="Proposed ticket" className="flex flex-col gap-2">
                  {proposed.description.trim() ? <Markdown>{proposed.description}</Markdown> : <EmptyState size="inline">No description.</EmptyState>}
                  {(proposed.priority !== 'none' || proposed.labels.length > 0) && (
                    <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
                      {proposed.priority !== 'none' && <span>{PRIORITY_LABEL[proposed.priority]} priority</span>}
                      <LabelChips labels={proposed.labels} className="flex-wrap" />
                    </div>
                  )}
                </div>
              )}
              {proposed.reason && <AgentMessage name={by} label={`Why ${by} proposed it`}><Markdown>{proposed.reason}</Markdown></AgentMessage>}
              <Checkbox checked={agentic} onChange={setAgentic} disabled={busy}>Label it agentic, so agents can pick it up</Checkbox>
              {duplicating && (
                <TicketPicker projectId={item.projectId} label="Duplicate of" placeholder="The ticket it repeats: key or title…"
                  exclude={item.ticketId ? [item.ticketId] : []} onPick={(t) => decide('duplicate', { duplicateOf: t.id })} />
              )}
            </>
          ),
          note: { label: 'Reason (optional)', placeholder: 'Why (optional): the agent gets it with your decision' },
          actions: [
            { id: 'accept_todo', title: 'Accept to Todo', short: 'To do', primary: true, pending: busy, disabled: busy || (editing && !draft.title.trim()), run: () => decide('accept_todo') },
            { id: 'accept_backlog', title: 'Accept to backlog', short: 'Backlog', disabled: busy || (editing && !draft.title.trim()), run: () => decide('accept_backlog') },
            { id: 'edit', title: 'Edit', expanded: editing, disabled: busy, run: () => setEditing((e) => !e) },
            { id: 'duplicate', title: 'Duplicate of…', short: 'Duplicate', expanded: duplicating, disabled: busy, run: () => setDuplicating((d) => !d) },
            { id: 'reject', title: 'Reject', end: true, danger: true, disabled: busy, run: () => decide('reject') },
          ],
          links: proposed.fromTicketId && item.projectId
            ? [{ label: `From ${proposed.fromTicket ?? 'ticket'}`, 'aria-label': `Open ${proposed.fromTicket ?? 'the ticket it came from'}`, onClick: () => openTicket({ id: proposed.fromTicketId!, projectId: item.projectId! }) }]
            : [],
        };
      }
      case 'confirm': {
        const { tool, agent, session, input } = confirmOf(item);
        if (rows) {
          // The PM's decisions (PDT-475): Allow applies the rows left ticked, as if each was clicked here.
          return decideSpec(rows, (kept) => void answer({ choice: 'allow', rows: kept }), () => plain('deny'), busy, session);
        }
        return {
          region: 'Agent call',
          why: `${session ?? 'An agent session'}${agent ? ` (${agent})` : ''} wants to call ${tool}. ${agent ?? 'Its agent'} can't ask in its own prompt, so PDT asks here; either way the agent hears what happened.`,
          content: <CodeCard aria-label="The call's arguments" title={<span className="font-mono">{tool}</span>}
            aside={[session ?? 'An agent session', agent].filter(Boolean).join(' · ')} code={input} />,
          actions: choiceActions(item, plain, busy),
        };
      }
      case 'gate': {
        const question = gateQuestion(item);
        const text = nonBlank(item.body.text);
        const from = workflow ?? 'Workflow';
        return {
          region: 'Gate',
          why: item.group === 'accept'
            ? `The work is done${workflow ? ` (${workflow})` : ''} and waits for your Accept before it goes on.`
            : `${workflow ? `Workflow ${workflow}` : 'A workflow'} waits for your choice.`,
          content: (
            <>
              {/* The whole question, when the title is only its step's name or first line (PDT-306: a pull request's review). */}
              {question && <AgentMessage aria-label="Question" name={from} label={from}><Markdown>{question}</Markdown></AgentMessage>}
              {/* What the gate is about, when its step says more than its question (PDT-288: a missing nightly). */}
              {text && <AgentMessage name={from} label={from}><Markdown>{text}</Markdown></AgentMessage>}
            </>
          ),
          note: {
            label: 'Note (optional)',
            placeholder: item.ticketId ? 'A note with your choice (optional), e.g. what to change when you reject it' : 'A note with your choice (optional)',
            hint: item.ticketId ? 'A note goes on the ticket as your comment, so the agent working on it gets it.' : undefined,
          },
          actions: choiceActions(item, withNote, busy),
        };
      }
    }
  })();
  return { spec: { ...spec, enter: spec.enter ?? true }, note, setNote };
}
