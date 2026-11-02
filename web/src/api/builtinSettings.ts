import { request } from './core';
import type { BuiltinSettingsView } from './gen/BuiltinSettingsView';

/** Settings backed by a built-in workflow's inputs: read, change some, reset to default. */
export interface BuiltinSettingsApi {
  get: () => Promise<BuiltinSettingsView>;
  save: (values: Record<string, unknown>) => Promise<BuiltinSettingsView>;
  reset: () => Promise<BuiltinSettingsView>;
}

/** The calls for one built-in's Settings route (`/api/pm-checkpoint`). */
export function builtinSettingsApi(path: string): BuiltinSettingsApi {
  return {
    get: () => request<BuiltinSettingsView>(path),
    save: (values) => request<BuiltinSettingsView>(path, { method: 'PUT', body: JSON.stringify({ values }) }),
    reset: () => request<BuiltinSettingsView>(`${path}/reset`, { method: 'POST' }),
  };
}
