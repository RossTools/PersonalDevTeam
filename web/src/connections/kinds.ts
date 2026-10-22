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
  /** Asks for a Discord server and its bot's application (a Discord bot). */
  discordBot?: boolean;
}

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
];

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
