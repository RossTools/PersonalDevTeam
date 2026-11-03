import { useProjects } from '../projects/queries';
import { usePersonas } from '../personas/model';

export interface ScopeOption {
  id: string;
  name: string;
}

/**
 * The projects and personas something can be limited to (a secret, an MCP server), as `CheckboxPicks` options, and a
 * name for each id. `host` names the host project (the PM's personal workflows); without it the host project is left
 * out. Archived personas are left out of the options but keep their names.
 */
export function useScopeOptions({ host }: { host?: string } = {}) {
  const projects = useProjects();
  const personas = usePersonas();
  const projectOptions: ScopeOption[] = (projects.data ?? [])
    .filter((p) => host !== undefined || p.kind !== 'host')
    .map((p) => ({ id: p.id, name: p.kind === 'host' ? host ?? p.name : p.name }));
  const personaOptions: ScopeOption[] = (personas.data ?? []).filter((p) => !p.archived).map((p) => ({ id: p.id, name: p.name }));
  return { projectOptions, personaOptions, projects: projects.data ?? [], personas: personas.data ?? [] };
}
