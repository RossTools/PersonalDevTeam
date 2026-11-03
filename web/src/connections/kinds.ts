// The kinds of connection the workflow actions use: what each needs, and how to get its secret. Each integration
// declares its own (crates/pdt-api/src/integrations/); the page reads them from the API. A connection's secret is kept
// in the OS keychain; PDT never shows it again.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { ConfigField } from '../api/gen/ConfigField';
import type { ConnectionKind } from '../api/gen/ConnectionKind';
import type { ConnectionLink } from '../api/gen/ConnectionLink';
import type { FieldWhen } from '../api/gen/FieldWhen';

export type { ConnectionKind, ConfigField };

const connectionKindsKey = ['connection-kinds'] as const;

/** Every kind of connection, from PDT's integrations (`GET /api/connections/kinds`); they change only with PDT. */
export function useConnectionKinds() {
  const q = useQuery({ queryKey: connectionKindsKey, queryFn: api.connectionKinds, staleTime: Infinity });
  const kinds = q.data ?? [];
  return { ...q, kinds, kindOf: (id: string) => kindOf(kinds, id), kindLabel: (id: string) => kindLabel(kinds, id) };
}

/** A setting's value as the dialog edits it (text). */
export function fieldText(f: ConfigField, v: unknown): string {
  if (f.type === 'lines') return Array.isArray(v) ? v.join('\n') : '';
  if (f.type === 'env') return v && typeof v === 'object' ? Object.entries(v).map(([k, x]) => `${k}=${String(x)}`).join('\n') : '';
  if (f.type === 'switch') return typeof v === 'boolean' ? String(v) : f.default ?? 'false';
  return typeof v === 'string' ? v : typeof v === 'number' ? String(v) : '';
}

/** Whether a condition holds, given the settings' text (`not` with `""`: once that setting is filled in). */
export const whenHolds = (w: FieldWhen, text: Record<string, string>) => ((text[w.key] ?? '').trim() === w.value) !== w.not;

/** Whether a setting is shown, given the others' text. */
export const fieldShown = (f: { when?: FieldWhen }, text: Record<string, string>) => !f.when || whenHolds(f.when, text);

/** Whether a setting's text matches its pattern (empty text always does: whether it is required is checked apart). */
export const fieldValid = (f: ConfigField, text: string) => !f.pattern || text.trim() === '' || new RegExp(f.pattern).test(text.trim());

/** The config the settings' text makes (empty and hidden settings left out). */
export function fieldsConfig(fields: ConfigField[], text: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of fields) {
    if (!fieldShown(f, text)) continue;
    const t = (text[f.key] ?? '').trim();
    if (f.type === 'switch') { out[f.key] = t === 'true'; continue; }
    if (!t) continue;
    if (f.type === 'lines') out[f.key] = t.split('\n').map((l) => l.trim()).filter(Boolean);
    else if (f.type === 'env') {
      out[f.key] = Object.fromEntries(t.split('\n').map((l) => l.trim()).filter((l) => l.includes('=')).map((l) => [l.slice(0, l.indexOf('=')).trim(), l.slice(l.indexOf('=') + 1)]));
    } else out[f.key] = t;
  }
  return out;
}

export const kindOf = (kinds: ConnectionKind[], id: string): ConnectionKind | undefined => kinds.find((k) => k.id === id);

const kindLabel = (kinds: ConnectionKind[], id: string) => kindOf(kinds, id)?.label ?? id;

export const AUTH_MODES = [
  { value: 'bearer', label: 'Bearer token' },
  { value: 'header', label: 'A header' },
  { value: 'basic', label: 'Basic (user and password)' },
  { value: 'query', label: 'A query parameter' },
  { value: 'none', label: 'Not sent' },
];

/** A kind's link with each `{key}` filled from the settings' text; null until each one it names is filled in and valid. */
export function linkUrl(link: ConnectionLink, fields: ConfigField[], text: Record<string, string>): string | null {
  let ok = true;
  const url = link.url.replace(/\{(\w+)\}/g, (_, key: string) => {
    const t = (text[key] ?? '').trim();
    const f = fields.find((x) => x.key === key);
    if (!t || (f && !fieldValid(f, t))) ok = false;
    return encodeURIComponent(t);
  });
  return ok ? url : null;
}

/** A kind's summary with each `{key}` filled from a connection's config; null when one has no text. */
export function summaryText(template: string, config: Record<string, unknown>): string | null {
  let ok = true;
  const text = template.replace(/\{(\w+)\}/g, (_, key: string) => {
    const v = config[key];
    if (typeof v !== 'string' || v.trim() === '') ok = false;
    return typeof v === 'string' ? v : '';
  });
  return ok ? text : null;
}

/** A random secret for an incoming webhook. */
export function randomSecret(): string {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Where another machine posts to start an incoming webhook's workflows. */
export const hookUrl = (name: string) => `${window.location.origin}/api/hooks/${encodeURIComponent(name)}`;
