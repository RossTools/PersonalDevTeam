import { enc, queryString, request } from './core';
import type { NewPersona } from './gen/NewPersona';
import type { Persona } from './gen/Persona';
import type { PersonaDiff } from './gen/PersonaDiff';
import type { PersonaPatch } from './gen/PersonaPatch';
import type { PersonaTemplate } from './gen/PersonaTemplate';
import type { PersonaTool } from './gen/PersonaTool';
import type { PersonaVersion } from './gen/PersonaVersion';
import type { SavePersona } from './gen/SavePersona';

/** Personas and their versions. */
export const personasApi = {
  listPersonas: () => request<Persona[]>('/api/personas'),
  personaTools: () => request<PersonaTool[]>('/api/personas/tools'),
  createPersona: (p: NewPersona) => request<Persona>('/api/personas', { method: 'POST', body: JSON.stringify(p) }),
  /** A new version. */
  savePersona: (id: string, p: SavePersona) =>
    request<Persona>(`/api/personas/${enc(id)}`, { method: 'PUT', body: JSON.stringify(p) }),
  patchPersona: (id: string, p: PersonaPatch) =>
    request<Persona>(`/api/personas/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deletePersona: (id: string) => request<void>(`/api/personas/${enc(id)}`, { method: 'DELETE' }),
  personaVersions: (id: string) => request<PersonaVersion[]>(`/api/personas/${enc(id)}/versions`),
  personaDiff: (id: string, from: number, to: number) =>
    request<PersonaDiff>(`/api/personas/${enc(id)}/diff${queryString({ from, to })}`),
  personaTemplate: (id: string) => request<PersonaTemplate>(`/api/personas/${enc(id)}/template`),
};
