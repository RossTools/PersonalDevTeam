import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Alert, Badge, Button, Dialog, Field, Input, Section, SettingsRow, Textarea, useConfirm } from '@ui';
import { api } from '../api/client';
import type { AcpAgentConfig } from '../api/gen/AcpAgentConfig';
import type { AcpAgentView } from '../api/gen/AcpAgentView';
import type { EnvVar } from '../api/gen/EnvVar';
import type { HostSettingsView } from '../api/gen/HostSettingsView';

/** Pure: an id for a new agent from its label ("Open Code" → `open-code`), not one of `taken`. */
export function idFor(label: string, taken: string[]): string {
  const base = label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 28) || 'agent';
  let id = base;
  for (let n = 2; taken.includes(id); n++) id = `${base}-${n}`;
  return id;
}

/** Pure: the arguments, one per line (blank lines dropped). */
export const parseArgs = (text: string): string[] => text.split('\n').map((l) => l.trim()).filter(Boolean);

/** Pure: `NAME=value` lines as variables; a line without `=` is an error (its line number, from 1). */
export function parseEnv(text: string): EnvVar[] | { badLine: number } {
  const vars: EnvVar[] = [];
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;
    const at = line.indexOf('=');
    if (at <= 0) return { badLine: i + 1 };
    vars.push({ name: line.slice(0, at).trim(), value: line.slice(at + 1) });
  }
  return vars;
}

const configOf = (a: AcpAgentView): AcpAgentConfig => ({ id: a.id, label: a.label, command: a.command, args: a.args, env: a.env });

/**
 * Settings → Agents → ACP agents (PDT-234): any agent that speaks the Agent Client Protocol on stdio, added by its
 * command, arguments and environment. Each is offered wherever an agent is (new tab, palette, Start, personas), only
 * as a conversation.
 */
export function AcpAgents({ v }: { v: HostSettingsView }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [editing, setEditing] = useState<AcpAgentView | 'new' | null>(null);
  const user = v.acpAgents.filter((a) => !a.builtin);
  const save = useMutation({
    mutationFn: (acpAgents: AcpAgentConfig[]) => api.saveHostSettings({ acpAgents }),
    onSuccess: (view) => {
      qc.setQueryData(['host-settings'], view);
      // Whether each one is found.
      void qc.invalidateQueries({ queryKey: ['host-checks'] });
    },
  });
  const remove = async (a: AcpAgentView) => {
    const { ok } = await confirm({
      title: `Remove ${a.label}?`,
      body: 'Its sessions stay but can no longer start it, and personas on it need another agent.',
      action: 'Remove agent',
      danger: true,
    });
    if (ok) save.mutate(user.filter((u) => u.id !== a.id).map(configOf));
  };
  return (
    <Section
      label="ACP agents"
      description="Other agents that speak the Agent Client Protocol (ACP). They run as conversations only."
      actions={<Button size="sm" onClick={() => setEditing('new')}>Add ACP agent</Button>}
    >
      {v.acpAgents.length === 0 && <p className="text-sm text-muted-foreground">None yet.</p>}
      {v.acpAgents.map((a) => (
        <SettingsRow
          key={a.id}
          label={a.label}
          description={<span className="font-mono break-all">{[a.command, ...a.args].join(' ')}</span>}
        >
          {a.found ? <Badge tone="running" title={a.found}>Found</Badge> : <Badge tone="failed">Missing</Badge>}
          {a.builtin ? (
            <Badge>Built in</Badge>
          ) : (
            <>
              <Button size="sm" onClick={() => setEditing(a)}>Edit</Button>
              <Button size="sm" variant="ghost" onClick={() => void remove(a)} disabled={save.isPending}>Remove</Button>
            </>
          )}
        </SettingsRow>
      ))}
      {save.error && !editing && <Alert tone="error">{save.error.message}</Alert>}
      {editing && (
        <AcpAgentDialog
          agent={editing === 'new' ? null : editing}
          taken={v.acpAgents.map((a) => a.id)}
          pending={save.isPending}
          error={save.error}
          onClose={() => { setEditing(null); save.reset(); }}
          onSave={(c) => {
            const others = user.filter((u) => editing === 'new' || u.id !== editing.id).map(configOf);
            save.mutate(editing === 'new' ? [...others, c] : user.map((u) => (u.id === editing.id ? c : configOf(u))), {
              onSuccess: () => setEditing(null),
            });
          }}
        />
      )}
    </Section>
  );
}

function AcpAgentDialog({ agent, taken, pending, error, onClose, onSave }: {
  agent: AcpAgentView | null;
  taken: string[];
  pending: boolean;
  error: Error | null;
  onClose: () => void;
  onSave: (c: AcpAgentConfig) => void;
}) {
  const [label, setLabel] = useState(agent?.label ?? '');
  const [command, setCommand] = useState(agent?.command ?? '');
  const [args, setArgs] = useState((agent?.args ?? []).join('\n'));
  const [env, setEnv] = useState((agent?.env ?? []).map((e) => `${e.name}=${e.value}`).join('\n'));
  const vars = parseEnv(env);
  const envError = 'badLine' in vars ? `Line ${vars.badLine} needs NAME=value.` : undefined;
  const ready = !!label.trim() && !!command.trim() && !envError;
  const save = () => {
    if (!ready || 'badLine' in vars) return;
    // The id stays once made: sessions and personas name the agent by it.
    const id = agent?.id ?? idFor(label, taken);
    onSave({ id, label: label.trim(), command: command.trim(), args: parseArgs(args), env: vars });
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      title={agent ? `Edit ${agent.label}` : 'Add ACP agent'}
      description="PDT starts the command in the session's folder and talks ACP over its stdin and stdout. Its modes come from the agent; when it has none, PDT keeps the session's permission mode itself."
      footer={<><Button variant="secondary" onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!ready} pending={pending} onClick={save}>{agent ? 'Save' : 'Add agent'}</Button></>}
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); save(); }}>
        <Field label="Name">
          <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="OpenCode" maxLength={40} />
        </Field>
        <Field label="Command" hint="A program on the PATH, a full path, or one starting with ~.">
          <Input mono spellCheck={false} value={command} onChange={(e) => setCommand(e.target.value)} placeholder="opencode" />
        </Field>
        <Field label="Arguments" hint="One per line: those that start its ACP server.">
          <Textarea mono spellCheck={false} rows={2} value={args} onChange={(e) => setArgs(e.target.value)} placeholder="acp" />
        </Field>
        <Field label="Environment" hint="NAME=value, one per line. Stored in this host's settings: don't put secrets here." error={envError}>
          <Textarea mono spellCheck={false} rows={2} value={env} onChange={(e) => setEnv(e.target.value)} />
        </Field>
        {error && <Alert tone="error">{error.message}</Alert>}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
