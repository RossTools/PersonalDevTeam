import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { NewRun } from '../api/gen/NewRun';
import type { RunKind } from '../api/gen/RunKind';

/** The runs and tests the effective configuration of one place offers, and any error in that configuration. */
export function useRunChoices(projectId: string, worktreeId: string | null) {
  const config = useQuery({
    queryKey: ['run-config', projectId, worktreeId],
    queryFn: () => api.runConfig(projectId, worktreeId),
  });
  const eff = config.data?.effective;
  const choices = eff ? [...eff.runs.map((d) => `run:${d.name}`), ...eff.tests.map((d) => `test:${d.name}`)] : [];
  return { choices, error: config.data?.error ?? null };
}

/** Splits a menu choice into the name and kind of a run. */
export function parseChoice(choice: string): Pick<NewRun, 'name' | 'kind'> {
  const [kind, ...name] = choice.split(':');
  return { name: name.join(':'), kind: kind as RunKind };
}

/** The one Run menu: a select over the configured runs and tests. */
export function RunMenu({ label, choices, value, onChange, disabled, className }: {
  label: string;
  choices: string[];
  value: string;
  onChange: (choice: string) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <select
      aria-label={label}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
      className={`rounded-pdt bg-surface border border-border text-xs text-muted-foreground px-1 py-0.5 ${className ?? ''}`}
    >
      <option value="">Run…</option>
      {choices.map((c) => (
        <option key={c} value={c}>{c.startsWith('test:') ? `Test ${c.slice(5)}` : c.slice(4)}</option>
      ))}
    </select>
  );
}
