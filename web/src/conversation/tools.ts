import type { ApprovalQuestion } from '@ui';
import { plural } from '../common/text';
import { isRecord } from '../common/guards';

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

/** A tool's own name: `mcp__pdt__search_tickets` → `search_tickets`. */
export function shortTool(tool: string): string {
  return /^mcp__.+?__(.+)$/.exec(tool)?.[1] ?? tool;
}

/**
 * "Ran 3 commands, edited 2 files": what a run of tool calls did (files counted once each). Calls of one tool that
 * fits no category read "search_tickets · 2 calls".
 */
export function summarizeTools(calls: { tool: string; title: string }[]): string {
  if (calls.length > 0 && calls.every((c) => c.tool === calls[0].tool) && category(calls[0].tool) === 'other') {
    return `${shortTool(calls[0].tool)} · ${plural(calls.length, 'call')}`;
  }
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
    counts.command && `ran ${plural(counts.command, 'command')}`,
    counts.edit && `edited ${plural(counts.edit, 'file')}`,
    counts.read && `read ${plural(counts.read, 'file')}`,
    counts.search && `searched ${plural(counts.search, 'time')}`,
    counts.web && `looked up ${plural(counts.web, 'page')}`,
    counts.agent && `ran ${plural(counts.agent, 'agent')}`,
    counts.other && `used ${plural(counts.other, 'tool')}`,
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

