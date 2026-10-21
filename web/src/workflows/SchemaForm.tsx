// A node's config form, generated from its kind's JSON Schema (schemars): text, numbers, switches, choices, lists of
// text and of objects, optional fields and Tpl fields (a literal or a `{{reference}}`). What it can't draw is a JSON
// field (Phase 7d spec §3.4).
import { useEffect, useState } from 'react';
import { Braces, Plus, Trash2 } from 'lucide-react';
import { Button, Field, IconButton, Input, MenuButton, Select, Switch, Textarea } from '@ui';
import { deref } from './model';

type Schema = Record<string, unknown>;
type Value = unknown;

/** `runKey` → "Run key". */
export function humanize(key: string): string {
  const words = key.replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/[_.]/g, ' ').toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

const isRef = (v: Value) => typeof v === 'string' && /\{\{.*\}\}/.test(v);

/** What a schema is, as far as the form cares. */
export type FieldKind =
  | { kind: 'string'; multiline: boolean }
  | { kind: 'number'; integer: boolean }
  | { kind: 'boolean' }
  | { kind: 'enum'; options: { value: string; description?: string }[] }
  | { kind: 'strings' }
  | { kind: 'objects'; item: Schema }
  | { kind: 'object'; schema: Schema }
  | { kind: 'any' }
  | { kind: 'json' };

export interface Classified {
  field: FieldKind;
  nullable: boolean;
  /** A literal or a reference (`Tpl<T>`). */
  tpl: boolean;
  description?: string;
}

const LONG = new Set(['text', 'body', 'question', 'description', 'message']);

function enumOptions(s: Schema, root: Schema): { value: string; description?: string }[] | null {
  if (Array.isArray(s.enum) && s.enum.every((x) => typeof x === 'string')) return (s.enum as string[]).map((value) => ({ value }));
  const variants = s.oneOf as Schema[] | undefined;
  if (!variants?.length) return null;
  const out: { value: string; description?: string }[] = [];
  for (const raw of variants) {
    const v = deref(raw, root);
    if (typeof v.const === 'string') out.push({ value: v.const, ...(typeof v.description === 'string' ? { description: v.description } : {}) });
    else if (Array.isArray(v.enum) && v.enum.every((x) => typeof x === 'string')) out.push(...(v.enum as string[]).map((value) => ({ value })));
    else return null;
  }
  return out;
}

export function classify(raw: Schema | boolean | undefined, root: Schema, key = ''): Classified {
  if (raw === true || raw === undefined) return { field: { kind: 'any' }, nullable: true, tpl: false };
  if (raw === false) return { field: { kind: 'json' }, nullable: true, tpl: false };
  let s = deref(raw, root);
  const description = typeof s.description === 'string' ? s.description : typeof raw.description === 'string' ? raw.description : undefined;
  let nullable = false;
  let tpl = false;
  // anyOf: [T, null] (Option) or [T, reference] (Tpl), possibly both.
  for (let i = 0; i < 3 && Array.isArray(s.anyOf); i++) {
    const variants = (s.anyOf as Schema[]).map((v) => deref(v, root));
    const rest = variants.filter((v) => {
      if (v.type === 'null') {
        nullable = true;
        return false;
      }
      if (v.type === 'string' && typeof v.pattern === 'string' && v.pattern.includes('\\{\\{')) {
        tpl = true;
        return false;
      }
      return true;
    });
    if (rest.length !== 1) return { field: { kind: rest.length === 0 ? 'any' : 'json' }, nullable, tpl, description };
    s = { ...rest[0], ...(s.default !== undefined ? { default: s.default } : {}) };
  }
  let type = s.type;
  if (Array.isArray(type)) {
    if (type.includes('null')) nullable = true;
    const t = type.filter((x) => x !== 'null');
    type = t.length === 1 ? t[0] : undefined;
  }
  const options = enumOptions(s, root);
  if (options) return { field: { kind: 'enum', options }, nullable, tpl, description };
  const base = { nullable, tpl, description };
  switch (type) {
    case 'string': return { field: { kind: 'string', multiline: LONG.has(key) }, ...base };
    case 'integer': return { field: { kind: 'number', integer: true }, ...base };
    case 'number': return { field: { kind: 'number', integer: false }, ...base };
    case 'boolean': return { field: { kind: 'boolean' }, ...base };
    case 'array': {
      const item = deref(s.items as Schema | undefined, root);
      if (item.type === 'string' && !enumOptions(item, root)) return { field: { kind: 'strings' }, ...base };
      if (item.type === 'object' || item.properties) return { field: { kind: 'objects', item }, ...base };
      return { field: { kind: 'json' }, ...base };
    }
    case 'object':
      return s.properties ? { field: { kind: 'object', schema: s }, ...base } : { field: { kind: 'json' }, ...base };
    default:
      return { field: { kind: s.properties ? 'json' : 'any' }, ...base };
  }
}

/** A value typed into an "any" field: JSON when it reads as a number, true/false/null, a list or an object. */
export function parseAny(text: string): Value {
  const t = text.trim();
  if (/^(-?\d+(\.\d+)?|true|false|null|\[.*\]|\{.*\})$/s.test(t)) {
    try {
      return JSON.parse(t);
    } catch {
      return text;
    }
  }
  return text;
}

const showAny = (v: Value) => (v === undefined ? '' : typeof v === 'string' ? v : JSON.stringify(v));

/** Inserts a `{{path}}` at the end of a text field. */
function RefPicker({ paths, onPick }: { paths: string[]; onPick: (ref: string) => void }) {
  if (!paths.length) return null;
  return (
    <MenuButton label="Insert a reference" size="sm" entries={() => paths.map((p) => ({ label: p, onSelect: () => onPick(`{{${p}}}`) }))}>
      <IconButton label="Insert a reference" size="sm"><Braces aria-hidden /></IconButton>
    </MenuButton>
  );
}

/** JSON in a text area: kept as typed until it parses. */
function JsonField({ value, onChange, label }: { value: Value; onChange: (v: Value) => void; label: string }) {
  const [text, setText] = useState(() => (value === undefined ? '' : JSON.stringify(value, null, 2)));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    // Follow outside changes unless the text already means the same.
    setText((t) => {
      try {
        if (JSON.stringify(JSON.parse(t)) === JSON.stringify(value)) return t;
      } catch { /* keep going */ }
      return value === undefined ? '' : JSON.stringify(value, null, 2);
    });
  }, [value]);
  return (
    <>
      <Textarea
        mono
        aria-label={label}
        rows={3}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          if (!e.target.value.trim()) return onChange(undefined);
          try {
            onChange(JSON.parse(e.target.value));
            setError(null);
          } catch (err) {
            setError((err as Error).message);
          }
        }}
      />
      {error && <span className="text-meta text-destructive">{error}</span>}
    </>
  );
}

interface ControlProps {
  c: Classified;
  value: Value;
  onChange: (v: Value) => void;
  label: string;
  root: Schema;
  refs: string[];
  readOnly?: boolean;
}

function Control({ c, value, onChange, label, root, refs, readOnly }: ControlProps) {
  const f = c.field;
  const clear = (v: Value) => onChange(v === '' && c.nullable ? undefined : v);
  // A Tpl field holding a reference shows it as text, with a button back to the literal.
  if (c.tpl && isRef(value) && f.kind !== 'string') {
    return (
      <div className="flex items-center gap-1">
        <Input mono aria-label={label} value={String(value)} readOnly={readOnly} onChange={(e) => onChange(e.target.value)} className="flex-1" />
        {!readOnly && <Button size="sm" variant="secondary" onClick={() => onChange(undefined)}>Use a value</Button>}
      </div>
    );
  }
  const refButton = c.tpl && !readOnly && f.kind !== 'string' && (
    <MenuButton label={`Use a reference for ${label}`} size="sm" entries={() => refs.map((p) => ({ label: p, onSelect: () => onChange(`{{${p}}}`) }))}>
      <IconButton label={`Use a reference for ${label}`} size="sm"><Braces aria-hidden /></IconButton>
    </MenuButton>
  );
  switch (f.kind) {
    case 'string': {
      const text = typeof value === 'string' ? value : value == null ? '' : String(value);
      const multiline = f.multiline || text.includes('\n');
      return (
        <div className="flex items-start gap-1">
          {multiline ? (
            <Textarea aria-label={label} rows={3} value={text} readOnly={readOnly} onChange={(e) => clear(e.target.value)} className="flex-1" />
          ) : (
            <Input aria-label={label} value={text} readOnly={readOnly} onChange={(e) => clear(e.target.value)} className="flex-1" />
          )}
          {!readOnly && <RefPicker paths={refs} onPick={(r) => onChange(text + r)} />}
        </div>
      );
    }
    case 'number':
      return (
        <div className="flex items-center gap-1">
          <Input
            aria-label={label}
            type="number"
            readOnly={readOnly}
            value={typeof value === 'number' ? String(value) : ''}
            onChange={(e) => {
              const t = e.target.value;
              if (t === '') return onChange(c.nullable ? undefined : 0);
              const n = Number(t);
              if (!Number.isNaN(n)) onChange(f.integer ? Math.trunc(n) : n);
            }}
            className="flex-1"
          />
          {refButton}
        </div>
      );
    case 'boolean':
      return (
        <div className="flex items-center gap-1">
          <Switch aria-label={label} checked={value === true} disabled={readOnly} onChange={(on) => onChange(on)} />
          {refButton}
        </div>
      );
    case 'enum': {
      const options = [...(c.nullable ? [{ value: '', label: '(none)' }] : []), ...f.options.map((o) => ({ value: o.value, label: humanize(o.value) }))];
      return (
        <div className="flex items-center gap-1">
          <Select aria-label={label} options={options} value={typeof value === 'string' ? value : ''} disabled={readOnly} onChange={(v) => onChange(v === '' ? undefined : v)} className="flex-1" />
          {refButton}
        </div>
      );
    }
    case 'strings': {
      const list = Array.isArray(value) ? (value as Value[]).map(showAny) : [];
      return (
        <div className="flex flex-col gap-1">
          {list.map((item, i) => (
            <div key={i} className="flex items-center gap-1">
              <Input aria-label={`${label} ${i + 1}`} value={item} readOnly={readOnly} onChange={(e) => onChange(list.map((x, j) => (j === i ? e.target.value : x)))} className="flex-1" />
              {!readOnly && <IconButton label={`Remove ${label} ${i + 1}`} size="sm" onClick={() => onChange(list.filter((_, j) => j !== i))}><Trash2 aria-hidden /></IconButton>}
            </div>
          ))}
          {!readOnly && <Button size="sm" variant="secondary" className="self-start" onClick={() => onChange([...list, ''])}><Plus aria-hidden /> Add</Button>}
        </div>
      );
    }
    case 'objects': {
      const list = Array.isArray(value) ? (value as Record<string, Value>[]) : [];
      return (
        <div className="flex flex-col gap-2">
          {list.map((item, i) => (
            <div key={i} className="flex flex-col gap-2 rounded-md border border-border p-2">
              <SchemaForm schema={f.item} root={root} value={item} refs={refs} readOnly={readOnly} label={`${label} ${i + 1}`}
                onChange={(v) => onChange(list.map((x, j) => (j === i ? v : x)))} />
              {!readOnly && <Button size="sm" variant="secondary" className="self-start" onClick={() => onChange(list.filter((_, j) => j !== i))}><Trash2 aria-hidden /> Remove</Button>}
            </div>
          ))}
          {!readOnly && (
            <Button size="sm" variant="secondary" className="self-start" onClick={() => onChange([...list, objectDefault(f.item, root)])}>
              <Plus aria-hidden /> Add
            </Button>
          )}
        </div>
      );
    }
    case 'object':
      return (
        <div className="flex flex-col gap-2 rounded-md border border-border p-2">
          <SchemaForm schema={f.schema} root={root} value={(value ?? {}) as Record<string, Value>} refs={refs} readOnly={readOnly} label={label}
            onChange={(v) => onChange(Object.keys(v).length ? v : c.nullable ? undefined : v)} />
        </div>
      );
    case 'any':
      return (
        <div className="flex items-center gap-1">
          <Input mono aria-label={label} value={showAny(value)} readOnly={readOnly} onChange={(e) => onChange(e.target.value === '' ? undefined : parseAny(e.target.value))} className="flex-1" />
          {!readOnly && <RefPicker paths={refs} onPick={(r) => onChange(r)} />}
        </div>
      );
    default:
      return <JsonField value={value} onChange={onChange} label={label} />;
  }
}

function objectDefault(schema: Schema, root: Schema): Record<string, Value> {
  const s = deref(schema, root);
  const props = (s.properties ?? {}) as Record<string, Schema>;
  const required = (s.required ?? []) as string[];
  return Object.fromEntries(required.map((k) => {
    const c = classify(props[k], root, k);
    const v = c.field.kind === 'boolean' ? false : c.field.kind === 'number' ? 0 : c.field.kind === 'strings' || c.field.kind === 'objects' ? []
      : c.field.kind === 'enum' ? c.field.options[0]?.value : '';
    return [k, v];
  }));
}

export interface SchemaFormProps {
  schema: Schema;
  /** Where `$ref`s resolve (the kind's whole config schema). */
  root?: Schema;
  value: Record<string, Value>;
  onChange: (v: Record<string, Value>) => void;
  /** `{{…}}` paths offered on text fields. */
  refs?: string[];
  readOnly?: boolean;
  /** Prefix for the fields' accessible names inside a list. */
  label?: string;
}

/** One field per property of an object schema. */
export function SchemaForm({ schema, root = schema, value, onChange, refs = [], readOnly, label }: SchemaFormProps) {
  const s = deref(schema, root);
  const props = (s.properties ?? {}) as Record<string, Schema>;
  const required = new Set((s.required ?? []) as string[]);
  const set = (k: string, v: Value) => {
    const next = { ...value };
    if (v === undefined) delete next[k];
    else next[k] = v;
    onChange(next);
  };
  const keys = Object.keys(props);
  if (!keys.length) return <p className="text-meta text-muted-foreground">Nothing to set.</p>;
  return (
    <>
      {keys.map((k) => {
        const c = classify(props[k], root, k);
        const name = humanize(k);
        const full = label ? `${label} ${name.toLowerCase()}` : name;
        return (
          <Field key={k} label={<>{name}{required.has(k) && <span className="text-muted-foreground"> *</span>}</>} hint={c.description}>
            <Control c={c} value={value[k]} onChange={(v) => set(k, v)} label={full} root={root} refs={refs} readOnly={readOnly} />
          </Field>
        );
      })}
    </>
  );
}
