// What an Inbox item shows and offers (PDT-361), whatever the screen: one line on why it waits, its content in the
// kind's style, the note box and the actions. The desktop and the phone lay the same spec out their own way.
import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import {
  AgentMessage, answerText, Callout, Checkbox, CodeCard, DiffViewer, EmptyState, Facts, Field, Input, NO_ANSWER, otherInputId, PassList, QuestionCard, requestAnswers, SectionLabel, Select, Textarea, type ApprovalAnswer, type QuestionAnswer,
} from '@ui';
import { cn } from '@ui/lib/utils';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { DecideRow } from '../api/gen/DecideRow';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Priority } from '../api/gen/Priority';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { LabelsEditor } from '../tickets/LabelsEditor';
import { LabelChips } from '../common/LabelChips';
import { TicketPicker } from '../tickets/TicketPicker';
import { approvalTitle, inputText, questionsOf as requestQuestions } from '../conversation/tools';
import { navigate } from '../shell/router';
import { useOpenSession, useOpenTicket } from '../common/open';
import { STORAGE_PATH, useCleanFinished } from '../projects/worktreeStorage';
import type { ItemLink } from './ItemLinks';
import {
  CHECKS_PATH, commandNoticeOf, confirmOf, CONNECT_PATH, decideRowsOf, dismissable, gateQuestion, MCP_SERVERS_PATH, noticeText, outcomeTone, passesOf, proposalOf,
  againstRecommendation, holdsTicket, inboxTargets, loginStepsOf, longTitle, needsConfirm, proposalPath, recommendationOf, recommendationTone, questionsOf, resends, ticketProposalOf, triageOf,
} from './model';
import { answerTiming, useAnswersHeld } from '../common/answerGuard';
import { answerApproval, approveCommand } from './approval';
import { DecideRows, useRowsDecision } from '../common/DecideRows';
import { pdtApproval } from '../conversation/PdtProposal';
import { pdtTool } from '../conversation/pdtTools';
import { nonBlank } from '../common/guards';
import { capitalize, formatBytes } from '../common/text';
import { commandText } from '../runs/ApproveCommands';
import { invalidateCommands } from '../runs/queries';
import { inboxKey } from './useInbox';

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
  /** The phone gives it a row of its own, under the rest (a long label: Always allow in this session). */
  wide?: boolean;
  /** A toggle (Take over): whether what it opens is open. */
  expanded?: boolean;
  /** Pressed once, it waits for a second press to act (PDT-623). */
  confirming?: boolean;
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
 * `answer` and `dismiss` act on it; `onApproved` runs once an approval is answered (held for Undo, PDT-623), `onUndo`
 * once that answer is undone. An action that acts outside PDT or can't be taken back asks for a second press
 * (`needsConfirm`), and every action is off for a moment after an answer (`useAnswersHeld`).
 */
export function useItemView({ item, busy, answer, dismiss, onApproved, onUndo, phone = false }: {
  item: InboxItem;
  busy: boolean;
  answer: (a: InboxAnswer) => Promise<void>;
  dismiss: () => void;
  onApproved: () => void;
  onUndo?: () => void;
  phone?: boolean;
}): { spec: ItemSpec; note: string; setNote: (s: string) => void } {
  const [note, setNote] = useState('');
  // A question's recommended choice starts picked (PDT-476); the user still sends it.
  // An approval's are the agent's own questions (AskUserQuestion, PDT-684).
  const [answers, setAnswers] = useState<QuestionAnswer[]>(() =>
    item.kind === 'question' ? questionsOf(item).map((q) => (q.recommended ? { choice: q.recommended, other: null } : NO_ANSWER))
      : item.kind === 'approval' ? requestQuestions(item.body.input).map(() => NO_ANSWER) : []);
  const [active, setActive] = useState(0);
  const [takingOver, setTakingOver] = useState(false);
  const [secret, setSecret] = useState('');
  const [clean, cleaning] = useCleanFinished();
  // A ticket proposal's edits, the agentic box and Duplicate of… (PDT-366).
  const [proposed] = useState(() => ticketProposalOf(item));
  const [editing, setEditing] = useState(false);
  const [duplicating, setDuplicating] = useState(false);
  const [agentic, setAgentic] = useState(false);
  const [draft, setDraft] = useState(() => ({ title: item.title, description: proposed.description, priority: proposed.priority, labels: proposed.labels }));
  // A batch call's rows (PDT-475): an Inbox confirmation's, or the PM conversation's card's (PDT-502).
  const rows = decideRowsOf(item);
  const decision = useRowsDecision(rows);
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  const targets = inboxTargets(item);
  const withNote = (choice: string) => void answer({ choice, ...(note.trim() ? { text: note } : {}) });
  const plain = (choice: string) => void answer({ choice });
  const dismissAction: ItemAction[] = dismissable(item) ? [{ id: 'dismiss', title: 'Dismiss', kbd: 'e', end: true, run: dismiss }] : [];
  const workflow = nonBlank(item.body.workflow);
  const persona = item.personaName ?? 'The agent';
  // A command waiting on the user's approval is approved in place (PDT-611); approving closes the notice.
  const qc = useQueryClient();
  const approve = (a: ApprovalAnswer, rows?: number[]) =>
    answerApproval(item, a, onApproved, { rows, phone, onUndo, then: () => qc.invalidateQueries({ queryKey: inboxKey }) });
  const waitingCommand = commandNoticeOf(item);
  // It trusts the command for every branch, so it is held for Undo, asks twice (`needsConfirm`) and Enter doesn't take it
  // (PDT-705).
  const trustCommand = () => {
    if (!waitingCommand) return;
    const { projectId, command } = waitingCommand;
    approveCommand(item, projectId, command.fingerprint, onApproved, {
      onUndo,
      then: async () => {
        void invalidateCommands(qc, projectId);
        await qc.invalidateQueries({ queryKey: inboxKey });
      },
    });
  };

  // The PM's batch of decisions, from an Inbox confirmation or its conversation's card: untick rows, then Allow (`y`)
  // or Deny (`n`), as on the card.
  const decideSpec = (rows: DecideRow[], allow: (kept: number[]) => void, deny: () => void, pending: boolean, session: string | null) => {
    const d = decision;
    return {
      region: 'Agent call',
      why: `${d.proposes(session ?? item.personaName ?? 'The PM')} ${d.help}`,
      content: <DecideRows rows={rows} kept={d.isKept} onToggle={d.toggle} disabled={pending} />,
      actions: [
        { id: 'allow', title: d.allowTitle, short: 'Allow', primary: true, kbd: 'y',
          pending, disabled: pending || !d.canAllow, run: () => allow(d.kept) },
        { id: 'deny', title: 'Deny', kbd: 'n', end: true, disabled: pending, run: deny },
      ],
      hint: d.ticked,
      enter: false,
      onKey: (key: string) => !pending && d.onKey(key, allow, deny),
    };
  };

  // A page of questions (PDT's `ask_user`, or the agent's own): each one's answer, and how many are answered.
  const questionState = (count: number) => {
    const at = (n: number) => answers[n] ?? NO_ANSWER;
    const set = (n: number, a: QuestionAnswer) => setAnswers(Array.from({ length: count }, (_, i) => (i === n ? a : at(i))));
    const answered = Array.from({ length: count }, (_, n) => at(n)).filter((a) => answerText(a).trim() || (a.choices?.length ?? 0) > 0).length;
    return { at, set, answered };
  };
  // A number key picks that choice of the question in hand (ticks it, for several), the last one types an own answer;
  // a single pick moves on to the next question.
  const numberKeys = (qs: { choices: string[]; multiple?: boolean }[]) => (key: string) => {
    const { at, set } = questionState(qs.length);
    const n = Math.min(active, qs.length - 1);
    const q = qs[n];
    const k = Number(key);
    if (!q || !Number.isInteger(k) || k < 1 || k > q.choices.length + 1 || q.choices.length === 0) return false;
    if (k > q.choices.length) {
      set(n, { ...at(n), other: at(n).other ?? '' });
      document.getElementById(otherInputId(`${item.id}-q${n}`))?.focus();
    } else if (q.multiple) {
      const label = q.choices[k - 1];
      const ticked = at(n).choices ?? [];
      set(n, { ...at(n), choices: ticked.includes(label) ? ticked.filter((l) => l !== label) : [...ticked, label] });
    } else {
      set(n, { choice: q.choices[k - 1], other: null });
      if (n + 1 < qs.length) setActive(n + 1);
    }
    return true;
  };

  const spec = ((): Omit<ItemSpec, 'enter'> & { enter?: boolean } => {
    switch (item.kind) {
      case 'question': {
        const qs = questionsOf(item);
        const { at, set, answered } = questionState(qs.length);
        const pickKey = numberKeys(qs.map((q) => ({ choices: q.choices.map((c) => c.label) })));
        return {
          region: 'Questions',
          why: `${persona} waits for your answers${item.ticketKey ? ` on ${item.ticketKey}` : ''}; its step carries on with them.${
            !item.ticketKey ? '' : holdsTicket(item) ? ` ${item.ticketKey} doesn't merge or move to Done until you answer.` : ` It doesn't hold ${item.ticketKey}'s merge.`}`,
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
          // The agent's own questions (AskUserQuestion): a question, as in its conversation, not a permission check
          // (PDT-576), laid out as PDT's own questions are (PDT-684): numbered choices, Send answers, Dismiss on `e`.
          const who = item.personaName ?? nonBlank(item.body.sessionTitle) ?? 'The agent';
          const { at, set, answered } = questionState(questions.length);
          const sent = requestAnswers(questions, at);
          const all = Object.values(sent).every((x) => x.length > 0);
          const send = () => { if (all) approve({ type: 'question', answers: sent }); };
          return {
            region: 'Questions',
            why: `${who} asks you in its conversation; the answers go back to it.`,
            content: questions.map((q, n) => (
              <QuestionCard key={n} id={`${item.id}-q${n}`} index={n} total={questions.length} label={q.question} numbers={!phone}
                question={questions.length === 1 && q.question === item.title && !longTitle(item) ? undefined
                  : <>{q.header && <SectionLabel as="span" className="mr-1.5">{q.header}</SectionLabel>}{q.question}</>}
                choices={q.options} multiple={q.multiSelect}
                value={at(n)} disabled={busy} onChange={(a) => { set(n, a); setActive(n); }} onSubmit={send} />
            )),
            actions: [
              { id: 'send', title: 'Send answers', short: 'Send', primary: true, disabled: !all, run: send },
              // It answers the agent (not PDT's Dismiss), so a phone keeps it in the bar.
              { id: 'decline', title: 'Dismiss', kbd: 'e', end: true, run: () => approve({ type: 'deny' }) },
            ],
            hint: `${answered} of ${questions.length} answered`,
            onKey: numberKeys(questions.map((q) => ({ choices: q.options.map((o) => o.label), multiple: q.multiSelect }))),
          };
        }
        const send = approve;
        const allow = () => send({ type: 'approve', always: false });
        const deny = () => send({ type: 'deny', ...(note.trim() ? { message: note.trim() } : {}) });
        if (rows) {
          // The PM's decisions, asked on its conversation's card (PDT-502): the same table, answered there too.
          return decideSpec(rows, (kept) => send({ type: 'approve', always: false }, kept), () => send({ type: 'deny' }), false,
            nonBlank(item.body.sessionTitle));
        }
        const where = [tool, nonBlank(item.body.sessionTitle)].filter(Boolean).join(' · ');
        // PDT's own tools say what they will create or change, as on the conversation's card (PDT-382).
        const pdt = diff ? null : pdtApproval(tool, item.body.input);
        const shown = diff || pdt ? '' : inputText(tool, item.body.input);
        const args = shown && shown !== '{}' && shown !== item.title ? shown : '';
        return {
          region: 'Approval', why, enter: false,
          content: pdt ? (
            <CodeCard aria-label={pdt.title} title={pdt.title} aside={nonBlank(item.body.sessionTitle) ?? undefined}><div className="p-2.5">{pdt.preview}</div></CodeCard>
          ) : (
            <CodeCard aria-label={approvalTitle(tool)} title={where} code={diff ? undefined : item.title}>
              {/* What the agent says the call is for, under the command: in the header it read as a button (PDT-686). */}
              {description && <p className={cn('px-3.5 text-xs text-muted-foreground break-words', diff ? 'border-b border-border py-2' : '-mt-1.5 pb-3')}>{description}</p>}
              {diff && <DiffViewer diff={diff} />}
              {/* Without a preview the arguments are all that says what the call does (PDT-541). */}
              {args && (
                <pre aria-label="All arguments" className="max-h-80 overflow-auto border-t border-border px-3.5 py-3 font-mono text-xs whitespace-pre-wrap break-words">{args}</pre>
              )}
            </CodeCard>
          ),
          note: { label: 'Reason (optional)', placeholder: 'Why not (optional): the agent gets it with Deny' },
          actions: [
            // The same words and order as the conversation's card (PDT-684).
            { id: 'allow', title: 'Allow', primary: true, kbd: 'y', run: allow },
            ...(item.choices.some((c) => c.id === 'allow_always')
              ? [{ id: 'always', title: 'Always allow in this session', wide: true, run: () => send({ type: 'approve', always: true }) }]
              : []),
            { id: 'deny', title: 'Deny', kbd: 'n', end: true, danger: true, run: deny },
          ],
          onKey: (key) => {
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
      case 'agent_login': {
        const agent = nonBlank(item.body.label) ?? 'The agent';
        const steps = loginStepsOf(item);
        const detail = nonBlank(item.body.detail);
        return {
          region: 'Not logged in',
          why: `${agent}'s steps wait instead of failing. Once it is logged in they go on by themselves (PDT checks every few minutes), or press Check again.`,
          content: (
            <>
              <Callout tone="warning"><Markdown>{nonBlank(item.body.hint) ?? `Log ${agent} in again.`}</Markdown></Callout>
              {steps.length > 0 && (
                <ul aria-label="Waiting steps" className="flex flex-col gap-1 text-sm">
                  {steps.map((s, n) => <li key={n}>{s.ticketKey ? `${s.persona} · ${s.ticketKey}` : s.persona}</li>)}
                </ul>
              )}
              {detail && <CodeCard aria-label="What it said" title="What it said" code={detail} />}
            </>
          ),
          actions: choiceActions(item, plain, busy),
          links: [{ label: 'Checks', onClick: () => navigate(CHECKS_PATH) }],
        };
      }
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
        if (waitingCommand) {
          const { command } = waitingCommand;
          return {
            region: 'Command to approve',
            why: "PDT runs a repository's commands only once you approve them. Approve runs this one from now on, in every branch of the project; it goes away once approved.",
            content: (
              <>
                <Callout tone="warning"><Markdown>{noticeText(item)}</Markdown></Callout>
                <CodeCard title={command.uses.join(', ') || 'Command'} aria-label="The command" code={commandText(command)} />
              </>
            ),
            actions: [{ id: 'approve', title: 'Approve', primary: true, run: trustCommand }, ...dismissAction],
            enter: false,
          };
        }
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
        const named = proposed.persona ?? item.personaName;
        // Mid-sentence an unnamed proposer is "the agent", not "An agent".
        const by = named ?? 'An agent';
        const them = named ?? 'the agent';
        const edits = editing ? { title: draft.title, description: draft.description, priority: draft.priority, labels: draft.labels } : {};
        // Reject takes only the reason; Accept and Duplicate of… the edits and the box.
        const decide = (choice: string, extra: Record<string, unknown> = {}) =>
          void answer({ choice, ...(choice === 'reject' ? {} : { proposal: { ...edits, agentic, ...extra } }), ...(note.trim() ? { text: note } : {}) });
        const set = (patch: Partial<typeof draft>) => setDraft((d) => ({ ...d, ...patch }));
        return {
          region: 'Ticket proposal',
          why: `${by} proposed this ticket${proposed.fromTicket ? ` while working on ${proposed.fromTicket}` : ''}. It waits in Draft, off the board, until you decide; ${them} hears what you choose.`,
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
              {proposed.reason && <AgentMessage name={by} label={`Why ${them} proposed it`}><Markdown>{proposed.reason}</Markdown></AgentMessage>}
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
        const { tool, agent, session, input, secretNote } = confirmOf(item);
        // Only PDT's own tools ask here, by their bare names.
        const pdt = pdtApproval(pdtTool(tool) ? tool : `mcp__pdt__${tool}`, item.body.input);
        if (rows) {
          // The PM's decisions (PDT-475): Allow applies the rows left ticked, as if each was clicked here.
          return decideSpec(rows, (kept) => void answer({ choice: 'allow', rows: kept }), () => plain('deny'), busy, session);
        }
        return {
          region: 'Agent call',
          why: `${session ?? 'An agent session'}${agent ? ` (${agent})` : ''} wants to call ${tool}. ${agent ?? 'Its agent'} can't ask in its own prompt, so PDT asks here; either way the agent hears what happened.${secretNote ? ` ${secretNote}` : ''}`,
          content: pdt
            ? <CodeCard aria-label={pdt.title} title={pdt.title} aside={[session ?? 'An agent session', agent].filter(Boolean).join(' · ')}><div className="p-2.5">{pdt.preview}</div></CodeCard>
            : <CodeCard aria-label="The call's arguments" title={<span className="font-mono">{tool}</span>}
              aside={[session ?? 'An agent session', agent].filter(Boolean).join(' · ')} code={input} />,
          note: { label: 'Reason (optional)', placeholder: 'Why not (optional): the agent gets it with Deny' },
          actions: choiceActions(item, (c) => (c === 'deny' ? withNote(c) : plain(c)), busy),
        };
      }
      case 'gate': {
        const question = gateQuestion(item);
        const recommendation = recommendationOf(item);
        const text = nonBlank(item.body.text);
        const from = workflow ?? 'Workflow';
        // A ticket stalled with nothing going on (PDT-510: its CI cancelled, or idle): it closes once something does.
        const stalled = item.group === 'stalled';
        return {
          region: stalled ? 'Stalled' : 'Gate',
          why: stalled
            ? `Nothing is moving this ticket on${workflow ? ` (found by ${workflow})` : ''}. It goes away by itself once something does.`
            : item.group === 'accept'
              ? `The work is done${workflow ? ` (${workflow})` : ''} and waits for your Accept before it goes on.`
              : `${workflow ? `Workflow ${workflow}` : 'A workflow'} waits for your choice.`,
          content: (
            <>
              {/* What the step before recommends leads (PDT-640: the reviewer said Close; another choice asks again). */}
              {recommendation && (
                <Callout tone={recommendationTone(recommendation.choice)} aria-label="Recommendation" title={`${recommendation.by} recommends: ${recommendation.title}`}>
                  Any other choice asks you to confirm it.
                </Callout>
              )}
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

  // An action that acts outside PDT or can't be taken back (Request changes, Close, Always allow) arms on its first
  // press and acts on a second (PDT-623), not one inside a double-click. It disarms after a few seconds, and says so for
  // a few more (PDT-667: the hint is what a screen reader hears).
  const [armed, setArmed] = useState<{ id: string; at: number } | null>(null);
  const [lapsed, setLapsed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    setLapsed(false);
    const t = setTimeout(() => { setArmed(null); setLapsed(true); }, 4000);
    return () => clearTimeout(t);
  }, [armed]);
  useEffect(() => {
    if (!lapsed) return;
    const t = setTimeout(() => setLapsed(false), 4000);
    return () => clearTimeout(t);
  }, [lapsed]);
  // Right after an answer the actions are off: a click meant for the answered item does nothing on this one.
  const held = useAnswersHeld();
  const actions = spec.actions.map((a): ItemAction => {
    const disabled = a.disabled || held;
    if (!needsConfirm(item, a.id)) return { ...a, disabled };
    if (armed?.id !== a.id) return { ...a, disabled, run: () => setArmed({ id: a.id, at: Date.now() }) };
    const confirm = () => {
      if (Date.now() - armed.at < answerTiming.cooldownMs) return;
      setArmed(null);
      a.run();
    };
    return { ...a, title: `Confirm ${a.title.charAt(0).toLowerCase()}${a.title.slice(1)}`, short: 'Confirm', danger: true, confirming: true, disabled, run: confirm };
  });
  const rec = recommendationOf(item);
  const hint = !armed
    ? lapsed ? 'Not confirmed: nothing was sent.' : spec.hint
    : rec && againstRecommendation(item, armed.id)
      ? `Press again to confirm: ${rec.by} recommends ${rec.title}.`
      : waitingCommand
        ? <>Press again to approve <code>{waitingCommand.command.command}</code>: it runs in every branch of the project.</>
        : 'Press again to confirm: this acts outside PDT or can’t be taken back.';
  // Its key asks twice too (`y` on an Allow that posts outside PDT).
  const onKey = (key: string) => {
    const a = actions.find((x) => x.kbd === key && needsConfirm(item, x.id));
    if (!a) return spec.onKey?.(key) ?? false;
    if (!a.disabled) a.run();
    return true;
  };
  return { spec: { ...spec, actions, hint, onKey, enter: spec.enter ?? true }, note, setNote };
}
