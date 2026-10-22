import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { Persona } from '../api/gen/Persona';
import type { PersonaAuthor } from '../api/gen/PersonaAuthor';
import type { PersonaSetup } from '../api/gen/PersonaSetup';

export const personaKeys = {
  all: ['personas'] as const,
  versions: (id: string) => ['personas', id, 'versions'] as const,
  diff: (id: string, from: number, to: number) => ['personas', id, 'diff', from, to] as const,
};

/** Every persona with its current version (archived ones too). */
export function usePersonas() {
  return useQuery({ queryKey: personaKeys.all, queryFn: api.listPersonas });
}

export const AGENT_LABEL: Record<AgentKind, string> = { claude: 'Claude', codex: 'Codex', devin: 'Devin' };
export const AGENTS: AgentKind[] = ['claude', 'codex', 'devin'];

export const AUTHOR_LABEL: Record<PersonaAuthor, string> = { user: 'You', builtin: 'PDT (built in)', agent: 'An agent' };

/** The pdt-mcp tools a persona may list, in pdt-mcp's order, with what each lets the agent do. */
export const TOOLS: { name: string; does: string }[] = [
  { name: 'get_ticket', does: 'Read the ticket, its comments and attempt' },
  { name: 'submit_outcome', does: 'Finish the step with an outcome and a summary' },
  { name: 'ask_user', does: 'Ask you questions (they wait in the Inbox)' },
  { name: 'add_ticket_comment', does: 'Comment on the ticket' },
  { name: 'add_review_comment', does: "Comment on a line of the attempt's changes" },
  { name: 'update_ticket', does: "Rewrite its own ticket's title and description" },
  { name: 'catalog_search', does: "Search the workflow node catalog" },
  { name: 'catalog_get', does: 'Read a catalog node' },
  { name: 'workflow_get', does: 'Read a workflow' },
  { name: 'workflow_validate', does: 'Check a workflow document' },
  { name: 'workflow_dry_run', does: 'Try a workflow without running it' },
  { name: 'workflow_save', does: 'Save a workflow (a new version)' },
  { name: 'get_overview', does: "See every project's board, tickets in flight, the Inbox and running sessions" },
  { name: 'search_tickets', does: 'Search tickets in every project' },
  { name: 'get_session', does: "Read a session's state and last messages" },
  { name: 'list_workflow_runs', does: 'List workflow runs and their steps' },
  { name: 'search_notes', does: "Search the PM's notes and journal" },
  { name: 'save_note', does: 'Save a note' },
  { name: 'delete_note', does: 'Delete a note' },
  { name: 'write_journal', does: "Write the session's journal entry" },
  { name: 'create_ticket', does: 'Create tickets (asks you first)' },
  { name: 'edit_ticket', does: 'Edit any ticket (asks you first)' },
  { name: 'send_to_session', does: 'Message a session (asks you first)' },
];
/** What an empty tool list means: the first five. */
export const DEFAULT_TOOLS = TOOLS.slice(0, 5).map((t) => t.name);

/** A tool list as checked on screen: an empty one is the default set. */
export const shownTools = (tools: string[]) => (tools.length ? tools : DEFAULT_TOOLS);

/** The tool list to save: the default set (in any order) is saved as empty, so it follows PDT's default. */
export function normalTools(tools: string[]): string[] {
  const ordered = TOOLS.map((t) => t.name).filter((n) => tools.includes(n));
  const isDefault = ordered.length === DEFAULT_TOOLS.length && DEFAULT_TOOLS.every((n) => ordered.includes(n));
  return isDefault ? [] : ordered;
}

/** The Select's value for "no mode of its own": the project's override, else the host default. */
export const DEFAULT_MODE = 'default';
export type ModeChoice = PermissionMode | typeof DEFAULT_MODE;

/** "Claude · opus" — the current version's agent and model, for list rows. */
export const agentLine = (p: Pick<Persona, 'agent' | 'model'>) => (p.model ? `${AGENT_LABEL[p.agent]} · ${p.model}` : AGENT_LABEL[p.agent]);

/** Active personas by name, then archived ones by name. */
export function sortPersonas(list: Persona[]): Persona[] {
  return [...list].sort((a, b) => Number(a.archived) - Number(b.archived) || a.name.localeCompare(b.name));
}

/** The setup fields of a persona (or one of its versions). */
export const setupOf = (p: PersonaSetup): PersonaSetup => ({
  agent: p.agent,
  model: p.model,
  instructions: p.instructions,
  permissionMode: p.permissionMode,
  tools: p.tools,
});

export function sameSetup(a: PersonaSetup, b: PersonaSetup): boolean {
  return (
    a.agent === b.agent &&
    (a.model ?? '') === (b.model ?? '') &&
    a.instructions === b.instructions &&
    a.permissionMode === b.permissionMode &&
    normalTools(a.tools).join(',') === normalTools(b.tools).join(',')
  );
}

/** A setup as sent: a blank model is the agent's default (null), the tools normalised. */
export const cleanSetup = (s: PersonaSetup): PersonaSetup => ({
  ...s,
  model: s.model?.trim() ? s.model.trim() : null,
  tools: normalTools(s.tools),
});
