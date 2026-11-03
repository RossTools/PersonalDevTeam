// Settings → MCP servers (PDT-322): MCP servers configured once in PDT, which every agent session gets (the PM,
// ticket and workflow agents, sessions the user starts), with secrets filled in from Settings → Secrets, an OAuth
// sign-in PDT holds for remote ones, and each tool allowed, asked or blocked. Servers are `mcp` connections, so
// workflows call the same ones.
import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Button, Checkbox, CheckboxPicks, Dialog, DialogActions, Field, Input, Page, Section, SegmentedControl, Select, Switch, Textarea, absoluteTime } from '@ui';
import { api } from '../api/client';
import type { AgentAccess } from '../api/gen/AgentAccess';
import type { McpProbe } from '../api/gen/McpProbe';
import type { McpServerView } from '../api/gen/McpServerView';
import type { OAuthSettings } from '../api/gen/OAuthSettings';
import type { ToolMode } from '../api/gen/ToolMode';
import { isWebUrl } from '../common/webUrl';
import { plural } from '../common/text';
import { hostKeys } from '../host/queries';
import { QueryStatus } from '../common/QueryStatus';
import { useConfirmDelete } from '../common/useConfirmDelete';
import { useScopeOptions } from '../common/useScopeOptions';
import { connectionsKey } from '../connections/queries';

export const mcpServersKey = ['mcp-servers'] as const;

const MODES: { value: ToolMode; label: string }[] = [
  { value: 'allow', label: 'Allow' },
  { value: 'ask', label: 'Ask' },
  { value: 'block', label: 'Block' },
];

const DEFAULT_ACCESS: AgentAccess = {
  enabled: true, pm: true, tickets: true, personas: [], user: true, projects: [], offIn: [], tools: { default: 'ask', overrides: {} },
};

/** One header or environment row: text, or a PDT secret by name. */
export interface Pair { key: string; secret: boolean; value: string }

export function pairsOf(v: unknown): Pair[] {
  if (!v || typeof v !== 'object') return [];
  return Object.entries(v as Record<string, unknown>).map(([key, x]) => {
    if (x && typeof x === 'object' && typeof (x as { $secret?: unknown }).$secret === 'string') return { key, secret: true, value: (x as { $secret: string }).$secret };
    const s = typeof x === 'string' ? x : '';
    return s.startsWith('secret:') ? { key, secret: true, value: s.slice(7) } : { key, secret: false, value: s };
  });
}

export function pairsValue(pairs: Pair[]): Record<string, unknown> {
  return Object.fromEntries(pairs.filter((p) => p.key.trim()).map((p) => [p.key.trim(), p.secret ? { $secret: p.value } : p.value]));
}

/** Name/value rows, each value text or one of PDT's secrets. */
function Pairs({ label, hint, pairs, secrets, onChange }: {
  label: string;
  hint: string;
  pairs: Pair[];
  secrets: string[];
  onChange: (p: Pair[]) => void;
}) {
  const set = (i: number, p: Partial<Pair>) => onChange(pairs.map((x, j) => (j === i ? { ...x, ...p } : x)));
  return (
    <Field label={label} hint={hint}>
      <div className="flex flex-col gap-1.5">
        {pairs.map((p, i) => (
          <div key={i} className="flex flex-wrap items-center gap-1.5">
            <Input aria-label={`${label} name`} className="w-40" value={p.key} onChange={(e) => set(i, { key: e.target.value })} />
            <SegmentedControl
              size="sm"
              aria-label={`${label} value kind`}
              options={[{ value: 'text', label: 'Text' }, { value: 'secret', label: 'Secret' }]}
              value={p.secret ? 'secret' : 'text'}
              onChange={(v) => set(i, { secret: v === 'secret', value: '' })}
            />
            {p.secret ? (
              <Select
                aria-label={`${label} secret`}
                placeholder="Pick a secret"
                options={secrets.map((s) => ({ value: s, label: s }))}
                value={p.value}
                onChange={(v) => set(i, { value: v })}
              />
            ) : (
              <Input aria-label={`${label} value`} className="min-w-0 flex-1" value={p.value} onChange={(e) => set(i, { value: e.target.value })} />
            )}
            <Button size="sm" variant="ghost" onClick={() => onChange(pairs.filter((_, j) => j !== i))}>Remove</Button>
          </div>
        ))}
        <div><Button size="sm" variant="secondary" onClick={() => onChange([...pairs, { key: '', secret: false, value: '' }])}>Add</Button></div>
      </div>
    </Field>
  );
}

/** `v` when it's a string, else ''. */
const stringOr = (v: unknown) => (typeof v === 'string' ? v : '');

/** Add a server, or edit one. */
export function McpServerDialog({ server, onClose }: { server?: McpServerView; onClose: () => void }) {
  const qc = useQueryClient();
  const c = server?.connection;
  const config = c?.config ?? {};
  const secrets = useQuery({ queryKey: ['secrets'], queryFn: api.listSecrets });
  const { projectOptions, personaOptions } = useScopeOptions();
  const [name, setName] = useState(c?.name ?? '');
  const [transport, setTransport] = useState<string>(server?.transport ?? 'stdio');
  const [command, setCommand] = useState(stringOr(config.command));
  const [args, setArgs] = useState(Array.isArray(config.args) ? (config.args as string[]).join('\n') : '');
  const [cwd, setCwd] = useState(stringOr(config.cwd));
  const [env, setEnv] = useState<Pair[]>(pairsOf(config.env));
  const [url, setUrl] = useState(c?.url ?? '');
  const [headers, setHeaders] = useState<Pair[]>(pairsOf(config.headers));
  const oauth0 = (config.oauth && typeof config.oauth === 'object' ? config.oauth : null) as OAuthSettings | null;
  const [useOauth, setUseOauth] = useState(oauth0 !== null);
  const [clientId, setClientId] = useState(oauth0?.clientId ?? '');
  const [clientSecret, setClientSecret] = useState(oauth0?.clientSecret ?? '');
  const [scopes, setScopes] = useState(oauth0?.scopes ?? '');
  const [access, setAccess] = useState<AgentAccess>(server?.access ?? DEFAULT_ACCESS);
  const remote = transport !== 'stdio';
  const save = useMutation({
    meta: { errorShown: true },
    mutationFn: () => {
      const own: Record<string, unknown> = remote
        ? { transport, headers: pairsValue(headers), oauth: useOauth ? { clientId: clientId.trim() || null, clientSecret: clientSecret || null, scopes: scopes.trim() || null } : null }
        : { transport, command: command.trim(), args: args.split('\n').map((a) => a.trim()).filter(Boolean), cwd: cwd.trim() || null, env: pairsValue(env) };
      const next = { ...config, ...own, agents: access };
      const body = { name: name.trim(), kind: 'mcp', url: remote ? url.trim() : '', config: next };
      return c ? api.updateConnection(c.id, body) : api.createConnection(body);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: mcpServersKey });
      void qc.invalidateQueries({ queryKey: connectionsKey });
      onClose();
    },
  });
  const secretNames = (secrets.data ?? []).map((s) => s.name);
  const ok = name.trim() !== '' && (remote ? /^https?:\/\/\S+$/.test(url.trim()) : command.trim() !== '');
  const set = (p: Partial<AgentAccess>) => setAccess((a) => ({ ...a, ...p }));
  return (
    <Dialog
      onClose={onClose}
      dismissable={false}
      title={c ? `Edit ${c.name}` : 'Add MCP server'}
      description="Every agent session it is for gets it, beside PDT's own tools. Secrets are filled in only in each session's launch settings."
      footer={<DialogActions onCancel={onClose} action="Save" disabled={!ok} pending={save.isPending} onAction={() => save.mutate()} />}
    >
      <div className="flex flex-col gap-3">
        <Field label="Name" hint="Agents see it under a short form of this name (Slack: mcp__slack__search).">
          <Input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Runs as">
          <SegmentedControl
            aria-label="Runs as"
            options={[{ value: 'stdio', label: 'A program' }, { value: 'http', label: 'An address' }, { value: 'sse', label: 'HTTP+SSE' }]}
            value={transport}
            onChange={setTransport}
          />
        </Field>
        {!remote && (
          <>
            <Field label="Program" hint="Found on PATH, or a full path. Each agent session starts its own.">
              <Input aria-label="Program" placeholder="npx" value={command} onChange={(e) => setCommand(e.target.value)} />
            </Field>
            <Field label="Arguments" hint="One per line.">
              <Textarea aria-label="Arguments" rows={3} value={args} onChange={(e) => setArgs(e.target.value)} />
            </Field>
            <Pairs label="Environment" hint="Values are text, or one of your secrets." pairs={env} secrets={secretNames} onChange={setEnv} />
            <Field label="Working folder" hint="Optional. Codex starts the program there; other agents start it in the session's folder.">
              <Input aria-label="Working folder" value={cwd} onChange={(e) => setCwd(e.target.value)} />
            </Field>
          </>
        )}
        {remote && (
          <>
            <Field label="Address">
              <Input aria-label="Address" placeholder="https://mcp.slack.com/mcp" value={url} onChange={(e) => setUrl(e.target.value)} />
            </Field>
            <Pairs label="Headers" hint="Values are text, or one of your secrets (an API key)." pairs={headers} secrets={secretNames} onChange={setHeaders} />
            <Checkbox checked={useOauth} onChange={setUseOauth}>Signs in with OAuth (PDT signs in once and gives agents the token)</Checkbox>
            {useOauth && (
              <>
                <Field label="Client id" hint="Optional: PDT registers itself when the service allows it. Otherwise register an app with the service and enter its client id.">
                  <Input aria-label="Client id" value={clientId} onChange={(e) => setClientId(e.target.value)} />
                </Field>
                <Field label="Client secret" hint="Optional: the app's secret, kept in Settings → Secrets.">
                  <Select aria-label="Client secret" placeholder="None" options={[{ value: '', label: 'None' }, ...secretNames.map((s) => ({ value: s, label: s }))]} value={clientSecret} onChange={setClientSecret} />
                </Field>
                <Field label="Scopes" hint="Optional, space-separated: the service's own list when empty.">
                  <Input aria-label="Scopes" value={scopes} onChange={(e) => setScopes(e.target.value)} />
                </Field>
              </>
            )}
          </>
        )}
        <Field label="Who gets it">
          <div className="flex flex-col gap-1">
            <Checkbox checked={access.enabled} onChange={(v) => set({ enabled: v })}>Agents (off: only workflows use it)</Checkbox>
            <Checkbox checked={access.pm} disabled={!access.enabled} onChange={(v) => set({ pm: v })}>The PM</Checkbox>
            <Checkbox checked={access.tickets} disabled={!access.enabled} onChange={(v) => set({ tickets: v })}>Ticket and workflow agents</Checkbox>
            <Checkbox checked={access.user} disabled={!access.enabled} onChange={(v) => set({ user: v })}>Sessions you start</Checkbox>
          </div>
        </Field>
        <CheckboxPicks label="Only these personas" hint="Workflow steps of every persona when none is ticked." options={personaOptions} value={access.personas} onChange={(personas) => set({ personas })} />
        <CheckboxPicks label="Only these projects" hint="Every project when none is ticked." options={projectOptions} value={access.projects} onChange={(projects) => set({ projects })} />
        <CheckboxPicks label="Turned off in" hint="Projects whose sessions don't get it." options={projectOptions} value={access.offIn} onChange={(offIn) => set({ offIn })} />
        <Field label="Its tools" hint="Allow runs without asking, Ask uses the agent's own prompt, Block makes a tool unavailable where the agent can. Set single tools after Test.">
          <SegmentedControl aria-label="Its tools" options={MODES} value={access.tools.default} onChange={(d) => set({ tools: { ...access.tools, default: d } })} />
        </Field>
        <p className="text-xs text-muted-foreground">Locked-down sessions (triage, reviews of outside pull requests) never get it.</p>
        {save.error && <Alert tone="error">{save.error.message}</Alert>}
      </div>
    </Dialog>
  );
}

/** What Test found: each tool with its mode, and the server's resources and prompts. */
function Probe({ server, probe }: { server: McpServerView; probe: McpProbe }) {
  const qc = useQueryClient();
  const policy = server.access.tools;
  const setMode = useMutation({
    meta: { errorShown: true },
    mutationFn: ({ tool, mode }: { tool: string; mode: ToolMode }) => {
      const overrides = { ...policy.overrides };
      if (mode === policy.default) delete overrides[tool];
      else overrides[tool] = mode;
      const agents = { ...server.access, tools: { ...policy, overrides } };
      return api.updateConnection(server.connection.id, { config: { ...server.connection.config, agents } });
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: mcpServersKey }),
  });
  if (probe.tools.error) return <Alert tone="error">{probe.tools.error}</Alert>;
  return (
    <div className="flex w-full flex-col gap-2 pl-2">
      <ul aria-label={`${server.connection.name} tools`} className="flex flex-col gap-1">
        {probe.tools.tools.map((t) => (
          <li key={t.name} className="flex flex-wrap items-center gap-2 text-sm">
            <code className="font-medium">{t.name}</code>
            <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{t.description}</span>
            <SegmentedControl
              size="sm"
              aria-label={`${t.name}: allow, ask or block`}
              options={MODES}
              value={policy.overrides[t.name] ?? policy.default}
              onChange={(mode) => setMode.mutate({ tool: t.name, mode })}
            />
          </li>
        ))}
        {probe.tools.tools.length === 0 && <li className="text-xs text-muted-foreground">It has no tools.</li>}
      </ul>
      {probe.resources.length > 0 && <p className="text-xs text-muted-foreground">Resources: {probe.resources.map((r) => r.name).join(', ')}</p>}
      {probe.prompts.length > 0 && <p className="text-xs text-muted-foreground">Prompts: {probe.prompts.map((p) => p.name).join(', ')}</p>}
      {setMode.error && <Alert tone="error">{setMode.error.message}</Alert>}
    </div>
  );
}

/** Who gets a server, in a line. */
export function audience(a: AgentAccess): string {
  if (!a.enabled) return 'Workflows only';
  const who = [a.pm && 'the PM', a.tickets && 'ticket agents', a.user && 'your sessions'].filter(Boolean);
  const where = a.projects.length ? ` in ${plural(a.projects.length, 'project')}` : '';
  return who.length ? `For ${who.join(', ')}${where}` : 'No agents';
}

/** Whether the browser may go to a sign-in address: only http(s), never `javascript:` or `data:` (it would run in
 * PDT's own page). PDT checks the service's addresses too; this holds if it ever misses one. */
export const signInUrlOk = isWebUrl;

function SignIn({ server }: { server: McpServerView }) {
  const qc = useQueryClient();
  const o = server.oauth;
  // The tab opens on the click (a tab opened after the request returns would be blocked as a popup), and goes to the
  // service once PDT says where.
  const signIn = useMutation({
    mutationFn: async (tab: Window | null) => {
      const start = await api.signInMcpServer(server.connection.id, window.location.origin);
      if (!signInUrlOk(start.url)) throw new Error("The service's sign-in address isn't a web address, so PDT won't open it.");
      return { tab, start };
    },
    onSuccess: ({ tab, start }) => {
      if (tab) {
        tab.opener = null;
        tab.location.href = start.url;
      } else window.location.assign(start.url);
    },
    onError: (_e, tab) => tab?.close(),
  });
  const signOut = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.signOutMcpServer(server.connection.id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: mcpServersKey }),
  });
  if (!o) return null;
  const status = !o.signedIn ? <Badge tone="waiting">Not signed in</Badge>
    : o.problem ? <Badge tone="failed">Sign-in failed: {o.problem}</Badge>
    : <span className="text-xs text-muted-foreground">Signed in{o.account ? ` as ${o.account}` : ''}{o.expiresAt ? ` · renews ${absoluteTime(o.expiresAt)}` : ''}</span>;
  const error = signIn.error ?? signOut.error;
  return (
    <div className="flex w-full flex-wrap items-center gap-2 pl-2">
      {status}
      <Button size="sm" variant={o.signedIn && !o.problem ? 'secondary' : 'primary'} pending={signIn.isPending} onClick={() => signIn.mutate(window.open('about:blank', '_blank'))}>
        {o.signedIn ? 'Sign in again' : 'Sign in'}
      </Button>
      {o.signedIn && <Button size="sm" variant="ghost" pending={signOut.isPending} onClick={() => signOut.mutate()}>Sign out</Button>}
      {signIn.isSuccess && <span className="text-xs text-muted-foreground">Finish signing in in the new tab, then come back here.</span>}
      {error && <Alert tone="error">{error.message}</Alert>}
    </div>
  );
}

function Row({ server, onEdit, onDelete }: { server: McpServerView; onEdit: () => void; onDelete: () => void }) {
  const [probe, setProbe] = useState<McpProbe | null>(null);
  const qc = useQueryClient();
  const test = useMutation({
    meta: { errorShown: true },
    mutationFn: () => api.testMcpServer(server.connection.id),
    onSuccess: (p) => { setProbe(p); void qc.invalidateQueries({ queryKey: mcpServersKey }); },
  });
  const kind = server.transport === 'stdio' ? 'Program' : server.transport === 'sse' ? 'HTTP+SSE' : 'Remote';
  return (
    <div className="flex flex-wrap items-center gap-2 py-1">
      <span className="min-w-0 truncate text-sm font-medium">{server.connection.name}</span>
      <code className="text-xs text-muted-foreground">{server.agentName}</code>
      <Badge>{kind}</Badge>
      <span className="min-w-0 truncate text-xs text-muted-foreground">{audience(server.access)}</span>
      <span className="flex-1" />
      <Button size="sm" variant="secondary" pending={test.isPending} onClick={() => test.mutate()}>Test</Button>
      <Button size="sm" variant="secondary" onClick={onEdit}>Edit</Button>
      <Button size="sm" variant="secondary" onClick={onDelete}>Delete</Button>
      <SignIn server={server} />
      {test.error && <Alert tone="error">{test.error.message}</Alert>}
      {probe && <Probe server={server} probe={probe} />}
    </div>
  );
}

export function McpServersPage() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: mcpServersKey, queryFn: api.mcpServers });
  const [editing, setEditing] = useState<{ server?: McpServerView } | null>(null);
  const remove = useConfirmDelete({
    remove: (s: McpServerView) => api.deleteConnection(s.connection.id),
    invalidate: [mcpServersKey],
    confirm: (s) => ({
      title: `Delete ${s.connection.name}?`,
      body: 'New agent sessions start without it, and workflows that call its tools fail. Its sign-in is forgotten.',
      action: 'Delete server',
    }),
  });
  const onlyPdt = useMutation({
    meta: { errorShown: true },
    mutationFn: (on: boolean) => api.saveHostSettings({ mcpOnlyPdt: on }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: mcpServersKey });
      void qc.invalidateQueries({ queryKey: hostKeys.settings });
    },
  });
  const error = remove.error ?? onlyPdt.error;
  return (
    <Page
      title="MCP servers"
      description="Add an MCP server once and every agent gets it: Claude Code, Codex and the other agents, in conversations and terminals. Remote servers that sign in with OAuth sign in here, once."
    >
      <QueryStatus query={list} what="MCP servers" skeleton="h-24">{({ servers, onlyPdt: onlyPdtOn }) => (
        <>
          <Section
            label="Servers"
            description="Test lists a server's tools; then allow, ask or block each one. What their tools return can hold text from other people: agents are told it isn't instructions."
            actions={<Button size="sm" onClick={() => setEditing({})}>Add server</Button>}
          >
            {servers.length
              ? servers.map((s) => <Row key={s.connection.id} server={s} onEdit={() => setEditing({ server: s })} onDelete={() => void remove.ask(s)} />)
              : <p className="text-sm text-muted-foreground">None yet.</p>}
          </Section>
          <Section label="Agents' own MCP servers" description="The MCP servers you added to each agent yourself keep loading beside these, unless you turn this on.">
            <Field
              inline
              label="Use only PDT's MCP servers"
              hint="Claude Code and Codex ignore the servers in their own settings. Other agents can't be told to, so theirs still load."
            >
              <Switch checked={onlyPdtOn} disabled={onlyPdt.isPending} onChange={(on) => onlyPdt.mutate(on)} />
            </Field>
          </Section>
        </>
      )}</QueryStatus>
      {error && <Alert tone="error">{error.message}</Alert>}
      {editing && <McpServerDialog server={editing.server} onClose={() => setEditing(null)} />}
    </Page>
  );
}
