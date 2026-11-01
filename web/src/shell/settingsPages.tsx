import type { ReactNode } from 'react';
import { McpServersPage } from '../mcp/McpServersPage';
import { NotificationSettingsPage } from '../notifications/NotificationSettingsPage';
import { AllConnectionsPage } from '../plugins/AllConnectionsPage';
import { PluginPage } from '../plugins/PluginPage';
import { PluginsPage } from '../plugins/PluginsPage';
import { SecretsPage } from '../secrets/SecretsPage';
import { routePath, type SettingsPage } from './route';
import { navigate } from './router';

/** The Settings pages of their own (not a project's sections or this host's): every kind but these three. */
export type OwnSettingsPage = Exclude<SettingsPage, { kind: 'index' | 'host' | 'project' }>;

/** What the shell gives a Settings page. */
export interface SettingsPageContext {
  phone: boolean;
  openPlugin: (id: string) => void;
  /** A plugin's name (once the plugins have loaded). */
  pluginTitle: (id: string) => string;
}

type Entry<P> = {
  /** A phone's top bar. */
  title: (page: P, ctx: SettingsPageContext) => string;
  screen: (page: P, ctx: SettingsPageContext) => ReactNode;
};

// Each Settings page of its own: a new one is its screen, its kind in route.ts (and SettingsNav), and one entry here.
const PAGES: { [K in OwnSettingsPage['kind']]: Entry<Extract<OwnSettingsPage, { kind: K }>> } = {
  connections: { title: () => 'All connections', screen: (_, ctx) => <AllConnectionsPage onOpen={ctx.openPlugin} /> },
  'mcp-servers': { title: () => 'MCP servers', screen: () => <McpServersPage /> },
  notifications: { title: () => 'Notifications', screen: () => <NotificationSettingsPage /> },
  plugins: {
    title: (page, ctx) => (page.id ? ctx.pluginTitle(page.id) : 'Plugins'),
    screen: (page, ctx) => (page.id
      ? <PluginPage key={page.id} id={page.id} phone={ctx.phone} onBack={() => navigate(routePath({ view: 'settings', page: { kind: 'plugins', id: null } }))} />
      : <PluginsPage phone={ctx.phone} onOpen={ctx.openPlugin} />),
  },
  secrets: { title: () => 'Secrets', screen: () => <SecretsPage /> },
};

const entry = (page: OwnSettingsPage) => PAGES[page.kind] as Entry<OwnSettingsPage>;

export const settingsPageTitle = (page: OwnSettingsPage, ctx: SettingsPageContext): string => entry(page).title(page, ctx);

export const settingsPageScreen = (page: OwnSettingsPage, ctx: SettingsPageContext): ReactNode => entry(page).screen(page, ctx);
