import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { AgentKind } from '../api/gen/AgentKind';
import type { PermissionMode } from '../api/gen/PermissionMode';
import type { Persona } from '../api/gen/Persona';
import type { PersonaAuthor } from '../api/gen/PersonaAuthor';
import type { PersonaSetup } from '../api/gen/PersonaSetup';
import type { PersonaTool } from '../api/gen/PersonaTool';

export const personaKeys = {
  all: ['personas'] as const,
  versions: (id: string) => ['personas', id, 'versions'] as const,
  diff: (id: string, from: number, to: number) => ['personas', id, 'diff', from, to] as const,
  template: (id: string, version: number) => ['personas', id, 'template', version] as const,
  tools: ['personas', 'tools'] as const,
};

/** Every persona with its current version (archived ones too). */
export function usePersonas() {
  return useQuery({ queryKey: personaKeys.all, queryFn: api.listPersonas });
}

export const AGENT_LABEL: Record<AgentKind, string> = { claude: 'Claude', codex: 'Codex', devin: 'Devin' };
export const AGENTS: AgentKind[] = ['claude', 'codex', 'devin'];

export const AUTHOR_LABEL: Record<PersonaAuthor, string> = { user: 'You', builtin: 'PDT (built in)', agent: 'An agent' };

/** Every pdt-mcp tool a persona may pick, from the server (every integration's), in pdt-mcp's order. */
export function usePersonaTools() {
  return useQuery({ queryKey: personaKeys.tools, queryFn: api.personaTools, staleTime: Infinity });
}

/** What an empty tool list means (pdt-core `DEFAULT_TOOLS`). */
export const DEFAULT_TOOLS = ['get_ticket', 'submit_outcome', 'ask_user', 'add_ticket_comment', 'add_review_comment'];

/** A tool list as checked on screen: an empty one is the default set. */
export const shownTools = (tools: string[]) => (tools.length ? tools : DEFAULT_TOOLS);

/** The tool list to save, without repeats: the default set (in any order) is saved as empty, so it follows PDT's
 *  default. It never drops a name, even one this page doesn't know (PDT-110). */
export function normalTools(tools: string[]): string[] {
  const unique = [...new Set(tools)];
  const isDefault = unique.length === DEFAULT_TOOLS.length && DEFAULT_TOOLS.every((n) => unique.includes(n));
  return isDefault ? [] : unique;
}

/** `tools` in the order of `known` (the server's list), names it doesn't list kept at the end. */
export const orderTools = (tools: string[], known: PersonaTool[]) => [
  ...known.map((t) => t.name).filter((n) => tools.includes(n)),
  ...tools.filter((n) => !known.some((t) => t.name === n)),
];

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
    [...normalTools(a.tools)].sort().join(',') === [...normalTools(b.tools)].sort().join(',')
  );
}

/** A setup as sent: a blank model is the agent's default (null), the tools normalised. */
export const cleanSetup = (s: PersonaSetup): PersonaSetup => ({
  ...s,
  model: s.model?.trim() ? s.model.trim() : null,
  tools: normalTools(s.tools),
});
