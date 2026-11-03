// Picks a connection by name (a template's connection param, a node's connection field), offering only those of the
// kind it needs, with Add… for a new one.
import { useState } from 'react';
import { Button, Select } from '@ui';
import { ConnectionDialog } from './ConnectionDialog';
import { useConnections } from './queries';
import { useConnectionKinds } from './kinds';

export function ConnectionPicker({ kind, value, onChange, label, disabled }: {
  /** A kind of connection's id; `any` or none: every connection. */
  kind?: string | null;
  value: string;
  onChange: (name: string) => void;
  label: string;
  disabled?: boolean;
}) {
  const list = useConnections();
  const { kindLabel, kindOf } = useConnectionKinds();
  const [adding, setAdding] = useState(false);
  const wanted = kind && kind !== 'any' ? kind : null;
  const fits = (list.data ?? []).filter((c) => !wanted || c.kind === wanted);
  const options = fits.map((c) => ({ value: c.name, label: wanted ? c.name : `${c.name} (${kindLabel(c.kind)})` }));
  // A name typed in by hand (or a reference) stays choosable.
  if (value && !options.some((o) => o.value === value)) options.push({ value, label: `${value} (not found)` });
  const placeholder = list.isLoading ? 'Loading…' : fits.length ? 'Choose a connection' : `No ${wanted ? kindLabel(wanted) : 'connection'} yet: add one`;
  return (
    <div className="flex items-center gap-1">
      <Select className="flex-1" aria-label={label} options={options} value={value || null} placeholder={placeholder} disabled={disabled} onChange={onChange} />
      {!disabled && <Button size="sm" variant="secondary" onClick={() => setAdding(true)}>Add…</Button>}
      {adding && (
        <ConnectionDialog
          kind={wanted && kindOf(wanted) ? wanted : undefined}
          onClose={() => setAdding(false)}
          onSaved={(c) => onChange(c.name)}
        />
      )}
    </div>
  );
}
