// Narrowing untyped JSON (tool inputs, event payloads, Inbox item bodies).

/** A plain object (not null, not an array). */
export const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** `v` when it's a string with something in it besides whitespace, else null. */
export const nonBlank = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);

/** The strings in `v` when it's an array (anything else in it dropped), else none. */
export const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
