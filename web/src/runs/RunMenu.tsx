import { useQuery } from '@tanstack/react-query';
import { Select } from '@ui';
import { api } from '../api/client';
import type { NewRun } from '../api/gen/NewRun';
import type { RunKind } from '../api/gen/RunKind';
import { runKeys } from './queries';

/**
 * The runs and tests the effective configuration of one place offers, and any error in that configuration. Off
 * (`enabled` false) where no Run menu shows, so a project without a repository (the PM's) is never asked.
 */
export function useRunChoices(projectId: string, worktreeId: string | null, enabled = true) {
  const config = useQuery({
    queryKey: runKeys.config(projectId, worktreeId),
    queryFn: () => api.runConfig(projectId, worktreeId),
    enabled: enabled && !!projectId,
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

/** A choice's label: "web" for a run, "Test unit" for a test. */
export const choiceLabel = (c: string) => (c.startsWith('test:') ? `Test ${c.slice(5)}` : c.slice(4));

/** The one Run menu: a select over the configured runs and tests, showing "Run…" until one is picked. */
export function RunMenu({ label, choices, value, onChange, disabled, className }: {
  label: string;
  choices: string[];
  value: string;
  onChange: (choice: string) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <Select
      aria-label={label}
      size="sm"
      placeholder="Run…"
      options={choices.map((c) => ({ value: c, label: choiceLabel(c) }))}
      value={value}
      disabled={disabled}
      onChange={onChange}
      className={className}
    />
  );
}
