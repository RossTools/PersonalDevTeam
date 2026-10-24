// The kinds of connection the workflow actions use (crates/pdt-api/src/workflow/nodes/): what each needs, and how to
// get its secret. A connection's secret is kept in the OS keychain; PDT never shows it again.

export interface ConnectionKind {
  id: string;
  label: string;
  /** Its section on the Connections page. */
  plural: string;
  /** How to set one up, under the section's title. */
  setup: string;
  /** What it is for, in the kind picker. */
  description: string;
  /** Asks for a base URL. */
  url: boolean;
  /** What the secret is, and where to find it. */
  secretLabel: string;
  secretHint: string;
  /** A random secret can be generated (an incoming webhook's). */
  generate?: boolean;
  /** Asks how the secret is sent (an HTTP service). */
  auth?: boolean;
  /** The secret may be left out. */
  secretOptional?: boolean;
  /** The address's example and hint, when it isn't a plain base URL. */
  urlPlaceholder?: string;
  urlHint?: string;
  /** The address may be left empty (then PDT uses the service's own). */
  urlOptional?: boolean;
  /** Asks for an address only when this setting has this value. */
  urlWhen?: { key: string; value: string };
  /** Settings of its own, kept in the connection's config. */
  fields?: ConfigField[];
  /** Asks for a Discord server and its bot's application (a Discord bot). */
  discordBot?: boolean;
  /** The PM acts through it: the switch that lets it act without asking (`config.pmActs`). */
  pmActs?: string;
}

/** One setting of a connection kind. */
export interface ConfigField {
  key: string;
  label: string;
  hint?: string;
  placeholder?: string;
  /** `text` (default); `lines`: a list, one per line; `env`: NAME=value lines; `choice`: one of `options`. */
  type?: 'text' | 'lines' | 'env' | 'choice';
  options?: { value: string; label: string }[];
  /** Shown only when another setting has this value. */
  when?: { key: string; value: string };
  /** May be left empty (otherwise a shown setting is required). */
  optional?: boolean;
}

const arrFields = (folder: string): ConfigField[] => [
  { key: 'qualityProfile', label: 'Quality profile', hint: 'Used when a step adds without naming one: a profile’s name or id. Empty: the first profile.', placeholder: 'HD-1080p', optional: true },
  { key: 'rootFolder', label: 'Root folder', hint: 'Where adds go unless a step says. Empty: the first root folder.', placeholder: folder, optional: true },
];

export const CONNECTION_KINDS: ConnectionKind[] = [
  {
    id: 'discord_webhook',
    label: 'Discord webhook',
    plural: 'Discord webhooks',
    setup: 'A channel that Post to Discord steps post to. In Discord, open the channel’s settings → Integrations → Webhooks → New Webhook, copy its URL, and add it here.',
    description: 'Posts to one Discord channel (Post to Discord).',
    url: false,
    secretLabel: 'Webhook URL',
    secretHint: 'In Discord: the channel’s settings (Edit Channel) → Integrations → Webhooks → New Webhook → Copy Webhook URL. It looks like https://discord.com/api/webhooks/…',
  },
  {
    id: 'discord_bot',
    label: 'Discord bot',
    plural: 'Discord bots',
    setup: 'A bot that lets the PM manage a Discord server: make and organise channels, and make the webhooks workflows post to. In the Discord Developer Portal, make an application, then under Bot reset its token and copy it. Invite the bot to your server with the link this dialog shows (Manage Channels, Manage Webhooks, Send Messages).',
    description: 'Lets the PM manage a Discord server’s channels and webhooks.',
    url: false,
    secretLabel: 'Bot token',
    secretHint: 'Discord Developer Portal → your application → Bot → Reset Token → Copy.',
    discordBot: true,
    pmActs: 'The PM changes this server without asking',
  },
  {
    id: 'webhook',
    label: 'Incoming webhook',
    plural: 'Incoming webhooks',
    setup: 'An address other machines post to, to start workflows with a Webhook trigger. Each has a secret the sender must send.',
    description: 'Lets another machine start workflows (the Webhook trigger).',
    url: false,
    secretLabel: 'Shared secret',
    secretHint: 'The sender sends it as Authorization: Bearer <secret>, or signs with it like GitHub (X-Hub-Signature-256).',
    generate: true,
  },
  {
    id: 'jira',
    label: 'Jira Cloud',
    plural: 'Jira Cloud sites',
    setup: 'A Jira Cloud site a project links to in its settings: issues assigned to you become tickets, and Done moves the issue.',
    description: 'A Jira Cloud site (projects link to it; Jira actions).',
    url: true,
    urlPlaceholder: 'https://your-site.atlassian.net',
    urlHint: 'Your Jira Cloud site.',
    fields: [{ key: 'email', label: 'Email', placeholder: 'you@work.com', hint: 'The Atlassian account’s email; PDT signs in as it.' }],
    secretLabel: 'API token',
    secretHint: 'At id.atlassian.com → Security → Create and manage API tokens → Create API token.',
  },
  {
    id: 'buildkite',
    label: 'Buildkite',
    plural: 'Buildkite organizations',
    setup: 'A Buildkite organization whose pipelines build a project’s branches (named in the project’s settings).',
    description: 'A Buildkite organization (CI of tickets’ branches; Buildkite actions).',
    url: true,
    urlOptional: true,
    urlPlaceholder: 'https://api.buildkite.com',
    urlHint: 'Leave empty for Buildkite’s own API.',
    fields: [{ key: 'org', label: 'Organization', placeholder: 'acme', hint: 'Its slug, as in buildkite.com/<organization>.' }],
    secretLabel: 'API token',
    secretHint: 'In Buildkite: Personal Settings → API Access Tokens → New API Access Token, with Read Builds (and Write Builds to retry or start builds).',
  },
  {
    id: 'http',
    label: 'HTTP service',
    plural: 'HTTP services',
    setup: 'A service HTTP request steps call by name: its address, and a token PDT sends only to it.',
    description: 'A service the HTTP request action calls: its address and token.',
    url: true,
    secretLabel: 'Token or password',
    secretHint: 'Sent only to this address, the way “Send it as” says.',
    auth: true,
  },
  {
    id: 'radarr',
    label: 'Radarr',
    plural: 'Radarr',
    setup: 'A Radarr that Search, Add and queue steps use for films.',
    description: 'Films: search, add, what’s downloading (the Radarr actions).',
    url: true,
    urlPlaceholder: 'http://192.168.1.10:7878',
    secretLabel: 'API key',
    secretHint: 'In Radarr: Settings → General → Security → API Key.',
    fields: arrFields('/movies'),
  },
  {
    id: 'sonarr',
    label: 'Sonarr',
    plural: 'Sonarr',
    setup: 'A Sonarr that Search, Add and queue steps use for series.',
    description: 'Series: search, add, what’s downloading (the Sonarr actions).',
    url: true,
    urlPlaceholder: 'http://192.168.1.10:8989',
    secretLabel: 'API key',
    secretHint: 'In Sonarr: Settings → General → Security → API Key.',
    fields: arrFields('/tv'),
  },
  {
    id: 'scryer',
    label: 'Scryer',
    plural: 'Scryer',
    setup: 'A Scryer media manager (films, series and anime): the PM browses its library, looks titles up and adds them, and Search, Add, queue and library steps use it.',
    description: 'Films, series and anime: library, search, add, what’s downloading (the PM and the Scryer actions).',
    url: true,
    urlPlaceholder: 'http://192.168.1.10:8080',
    urlHint: 'Scryer’s address, with its base path if it has one (http://nas/scryer).',
    secretLabel: 'API key',
    secretHint: 'In Scryer: Settings → API keys → Create. It starts with ska_.',
    pmActs: 'The PM adds to this library without asking',
  },
  {
    id: 'plex',
    label: 'Plex server',
    plural: 'Plex servers',
    setup: 'A Plex Media Server that Search, Recently added and Refresh steps use.',
    description: 'A Plex library: search, what’s new, refresh (the Plex actions).',
    url: true,
    urlPlaceholder: 'http://192.168.1.10:32400',
    secretLabel: 'Plex token',
    secretHint: 'In Plex Web: an item’s ⋯ → Get Info → View XML; the address that opens ends in X-Plex-Token=<token>.',
  },
  {
    id: 'mcp',
    label: 'MCP server',
    plural: 'MCP servers',
    setup: 'A Model Context Protocol server whose tools workflows call (each tool is an action in the catalog). PDT starts a program’s server when it is first used and stops it after 5 minutes idle.',
    description: 'An MCP server: its tools become workflow actions.',
    url: false,
    urlPlaceholder: 'https://example.com/mcp',
    urlWhen: { key: 'transport', value: 'http' },
    secretLabel: 'Secret',
    secretHint: 'Optional. A program gets it in the environment variable named below; an HTTP server gets it as a bearer token.',
    secretOptional: true,
    fields: [
      { key: 'transport', label: 'Runs as', type: 'choice', options: [{ value: 'stdio', label: 'A program PDT starts (stdio)' }, { value: 'http', label: 'An address (HTTP)' }] },
      { key: 'command', label: 'Program', placeholder: 'npx', hint: 'Found on PATH, or a full path. On Windows, a .cmd (npx) needs cmd with /c first.', when: { key: 'transport', value: 'stdio' } },
      { key: 'args', label: 'Arguments', type: 'lines', placeholder: '-y\n@modelcontextprotocol/server-filesystem\n/Users/me/notes', hint: 'One per line.', when: { key: 'transport', value: 'stdio' }, optional: true },
      { key: 'env', label: 'Environment', type: 'env', placeholder: 'LOG_LEVEL=warn', hint: 'NAME=value, one per line.', when: { key: 'transport', value: 'stdio' }, optional: true },
      { key: 'secretEnv', label: 'Secret’s variable', placeholder: 'API_TOKEN', hint: 'The environment variable the secret is passed in.', when: { key: 'transport', value: 'stdio' }, optional: true },
    ],
  },
];

/** A setting's value as the dialog edits it (text). */
export function fieldText(f: ConfigField, v: unknown): string {
  if (f.type === 'lines') return Array.isArray(v) ? v.join('\n') : '';
  if (f.type === 'env') return v && typeof v === 'object' ? Object.entries(v).map(([k, x]) => `${k}=${String(x)}`).join('\n') : '';
  return typeof v === 'string' ? v : typeof v === 'number' ? String(v) : '';
}

/** Whether a setting is shown, given the others' text. */
export const fieldShown = (f: { when?: { key: string; value: string } }, text: Record<string, string>) => !f.when || text[f.when.key] === f.when.value;

/** The config the settings' text makes (empty and hidden settings left out). */
export function fieldsConfig(fields: ConfigField[], text: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of fields) {
    if (!fieldShown(f, text)) continue;
    const t = (text[f.key] ?? '').trim();
    if (!t) continue;
    if (f.type === 'lines') out[f.key] = t.split('\n').map((l) => l.trim()).filter(Boolean);
    else if (f.type === 'env') {
      out[f.key] = Object.fromEntries(t.split('\n').map((l) => l.trim()).filter((l) => l.includes('=')).map((l) => [l.slice(0, l.indexOf('=')).trim(), l.slice(l.indexOf('=') + 1)]));
    } else out[f.key] = t;
  }
  return out;
}

export const kindOf = (id: string): ConnectionKind | undefined => CONNECTION_KINDS.find((k) => k.id === id);

export const kindLabel = (id: string) => kindOf(id)?.label ?? id;

export const AUTH_MODES = [
  { value: 'bearer', label: 'Bearer token' },
  { value: 'header', label: 'A header' },
  { value: 'basic', label: 'Basic (user and password)' },
  { value: 'query', label: 'A query parameter' },
  { value: 'none', label: 'Not sent' },
];

/** What a Discord bot needs: View Channels, Manage Channels, Send Messages, Manage Webhooks. */
export const DISCORD_BOT_PERMISSIONS = String(1024 + 16 + 2048 + 536870912);

/** The link that adds an application's bot to a server with the permissions PDT needs. */
export const discordInviteUrl = (applicationId: string) =>
  `https://discord.com/oauth2/authorize?client_id=${encodeURIComponent(applicationId)}&scope=bot&permissions=${DISCORD_BOT_PERMISSIONS}`;

/** A random secret for an incoming webhook. */
export function randomSecret(): string {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Where another machine posts to start an incoming webhook's workflows. */
export const hookUrl = (name: string) => `${window.location.origin}/api/hooks/${encodeURIComponent(name)}`;
