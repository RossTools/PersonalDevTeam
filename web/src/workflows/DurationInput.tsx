// A length of time for a `duration` template param: an amount and a unit, kept as milliseconds. Inside a Field the
// amount takes the Field's label; the unit is named after it.
import { useState } from 'react';
import { Input, Select } from '@ui';

const UNITS = [
  { value: 'd', label: 'days', ms: 86_400_000 },
  { value: 'h', label: 'hours', ms: 3_600_000 },
  { value: 'm', label: 'minutes', ms: 60_000 },
] as const;

type Unit = (typeof UNITS)[number]['value'];

/** The largest unit `ms` is a whole number of (minutes when none is). */
function unitOf(ms: number): Unit {
  return (UNITS.find((u) => ms > 0 && ms % u.ms === 0) ?? UNITS[2]).value;
}

export function DurationInput({ label, value, onChange }: { label: string; value: unknown; onChange: (ms: number | '') => void }) {
  const ms = typeof value === 'number' ? value : null;
  const [unit, setUnit] = useState<Unit>(() => unitOf(ms ?? 0));
  const scale = UNITS.find((u) => u.value === unit)!.ms;
  const amount = ms === null ? '' : String(ms / scale);
  return (
    <div className="flex gap-2">
      <Input type="number" min={0} value={amount} className="w-28"
        onChange={(e) => onChange(e.target.value === '' ? '' : Math.max(0, Math.round(Number(e.target.value) * scale)))} />
      <Select aria-label={`${label} unit`} options={UNITS.map((u) => ({ value: u.value, label: u.label }))} value={unit}
        onChange={(u) => {
          const next = UNITS.find((x) => x.value === u)!;
          setUnit(next.value);
          if (ms !== null) onChange(Math.round((ms / scale) * next.ms));
        }} />
    </div>
  );
}
