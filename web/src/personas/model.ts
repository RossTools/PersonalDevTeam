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

export const AUTHOR_LABEL: Record<PersonaAuthor, string> = { user: 'You', builtin: 'PDT (built in)', agent: 'An agent' };

/** Every pdt-mcp tool a persona may pick, from the server (every integration's), in pdt-mcp's order. */
export function usePersonaTools() {
  return useQuery({ queryKey: personaKeys.tools, queryFn: api.personaTools, staleTime: Infinity });
}

/** The default set (what a session with no persona gets, and what a new persona starts with), from the server. */
export const defaultTools = (known: PersonaTool[]) => known.filter((t) => t.default).map((t) => t.name);

/** The tool list to save, without repeats (empty: no tools). It never drops a name, even one this page doesn't know
 *  (PDT-110). */
export const normalTools = (tools: string[]) => [...new Set(tools)];

/** `tools` in the order of `known` (the server's list), names it doesn't list kept at the end. */
export const orderTools = (tools: string[], known: PersonaTool[]) => [
  ...known.map((t) => t.name).filter((n) => tools.includes(n)),
  ...tools.filter((n) => !known.some((t) => t.name === n)),
];

/** The Select's value for "no mode of its own": the project's override, else the host default. */
export const DEFAULT_MODE = 'default';
export type ModeChoice = PermissionMode | typeof DEFAULT_MODE;

/** "Claude · opus" — the current version's agent (named by `label`, `useAgents().label`) and model, for list rows. */
export const agentLine = (p: Pick<Persona, 'agent' | 'model'>, label: (a: AgentKind) => string) =>
  p.model ? `${label(p.agent)} · ${p.model}` : label(p.agent);

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
