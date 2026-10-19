import type { ApprovalQuestion } from '@ui';

/** The verb a tool call row starts with ("Run `cargo test`"), by the agent's tool name. */
export function toolVerb(tool: string): string {
  switch (tool) {
    case 'Bash':
    case 'BashOutput':
      return 'Run';
    case 'Read':
      return 'Read';
    case 'Edit':
    case 'MultiEdit':
    case 'NotebookEdit':
      return 'Edit';
    case 'Write':
      return 'Write';
    case 'Grep':
    case 'Glob':
      return 'Search';
    case 'WebFetch':
      return 'Fetch';
    case 'WebSearch':
      return 'Search the web';
    case 'Task':
    case 'Agent':
      return 'Agent:';
    case 'TodoWrite':
      return 'Update to-dos';
    default:
      return mcpName(tool) ?? tool;
  }
}

/** `mcp__server__tool` → "server: tool". */
function mcpName(tool: string): string | null {
  const m = /^mcp__(.+?)__(.+)$/.exec(tool);
  return m ? `${m[1]}: ${m[2]}` : null;
}

type Category = 'command' | 'edit' | 'read' | 'search' | 'web' | 'agent' | 'other';

function category(tool: string): Category {
  switch (toolVerb(tool)) {
    case 'Run':
      return 'command';
    case 'Edit':
    case 'Write':
      return 'edit';
    case 'Read':
      return 'read';
    case 'Search':
      return 'search';
    case 'Fetch':
    case 'Search the web':
      return 'web';
    case 'Agent:':
      return 'agent';
    default:
      return 'other';
  }
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "Ran 3 commands, edited 2 files": what a run of tool calls did (files counted once each). */
export function summarizeTools(calls: { tool: string; title: string }[]): string {
  const counts: Record<Category, number> = { command: 0, edit: 0, read: 0, search: 0, web: 0, agent: 0, other: 0 };
  const files: Partial<Record<Category, Set<string>>> = {};
  for (const c of calls) {
    const cat = category(c.tool);
    if ((cat === 'edit' || cat === 'read') && c.title) {
      (files[cat] ??= new Set()).add(c.title);
    } else counts[cat]++;
  }
  counts.edit += files.edit?.size ?? 0;
  counts.read += files.read?.size ?? 0;
  const parts = [
    counts.command && `ran ${plural(counts.command, 'command', 'commands')}`,
    counts.edit && `edited ${plural(counts.edit, 'file', 'files')}`,
    counts.read && `read ${plural(counts.read, 'file', 'files')}`,
    counts.search && `searched ${plural(counts.search, 'time', 'times')}`,
    counts.web && `looked up ${plural(counts.web, 'page', 'pages')}`,
    counts.agent && `ran ${plural(counts.agent, 'agent', 'agents')}`,
    counts.other && `used ${plural(counts.other, 'tool', 'tools')}`,
  ].filter(Boolean) as string[];
  const text = parts.join(', ');
  return text ? text[0].toUpperCase() + text.slice(1) : 'No tools';
}

/** The approval card's question for a tool ("Run this command?"). */
export function approvalTitle(tool: string): string {
  switch (category(tool)) {
    case 'command':
      return 'Run this command?';
    case 'edit':
      return tool === 'Write' ? 'Write this file?' : 'Edit this file?';
    case 'read':
      return 'Read this file?';
    case 'search':
      return 'Search for this?';
    case 'web':
      return tool === 'WebFetch' ? 'Fetch this page?' : 'Search the web for this?';
    case 'agent':
      return 'Start this agent?';
    default:
      return `Use ${toolVerb(tool)}?`;
  }
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Claude's AskUserQuestion input → the card's questions (unknown shapes give none). */
export function questionsOf(input: unknown): ApprovalQuestion[] {
  if (!isRecord(input) || !Array.isArray(input.questions)) return [];
  return input.questions.flatMap((q): ApprovalQuestion[] => {
    if (!isRecord(q) || typeof q.question !== 'string') return [];
    const options = Array.isArray(q.options)
      ? q.options.flatMap((o) =>
          isRecord(o) && typeof o.label === 'string'
            ? [{ label: o.label, ...(typeof o.description === 'string' ? { description: o.description } : {}) }]
            : typeof o === 'string' ? [{ label: o }] : [],
        )
      : [];
    return [{
      question: q.question,
      ...(typeof q.header === 'string' ? { header: q.header } : {}),
      options,
      multiSelect: q.multiSelect === true,
    }];
  });
}

/** ExitPlanMode's plan text, if its input has one. */
export function planOf(input: unknown): string | null {
  return isRecord(input) && typeof input.plan === 'string' ? input.plan : null;
}

/** A tool call's input as text for its expanded row: a Bash command as is, anything else as pretty JSON. */
export function inputText(tool: string, input: unknown): string {
  if (isRecord(input) && tool === 'Bash' && typeof input.command === 'string') return input.command;
  if (input === null || input === undefined) return '';
  try {
    return JSON.stringify(input, null, 2);
  } catch {
    return String(input);
  }
}

/** "0:42", "12:05", "1:02:03". */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

/** "850 ms", "12 s", "3 min 4 s". */
export function duration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return s % 60 ? `${m} min ${s % 60} s` : `${m} min`;
}
