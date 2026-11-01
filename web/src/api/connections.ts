import { enc, request } from './core';
import type { Connection } from './gen/Connection';
import type { ConnectionInput } from './gen/ConnectionInput';
import type { ConnectionKind } from './gen/ConnectionKind';
import type { ConnectionStatus } from './gen/ConnectionStatus';
import type { IntegrationInfo } from './gen/IntegrationInfo';
import type { McpProbe } from './gen/McpProbe';
import type { McpServersView } from './gen/McpServersView';
import type { McpTools } from './gen/McpTools';
import type { ProjectSettingsSchema } from './gen/ProjectSettingsSchema';
import type { SignInStart } from './gen/SignInStart';

/** Connections, plugins (integrations) and MCP servers. */
export const connectionsApi = {
  listConnections: () => request<Connection[]>('/api/connections'),
  connectionKinds: () => request<ConnectionKind[]>('/api/connections/kinds'),
  projectSettingsSchemas: () => request<ProjectSettingsSchema[]>('/api/integrations/project-settings'),
  listIntegrations: () => request<IntegrationInfo[]>('/api/integrations'),
  /** Enables exactly `enabled` of the optional plugins and turns off `off` of PDT's own (the rest of them are on). */
  setIntegrations: (enabled: string[], off: string[]) =>
    request<IntegrationInfo[]>('/api/integrations', { method: 'PUT', body: JSON.stringify({ enabled, off }) }),
  /** Runs an enabled plugin's checks now (Test again), then lists the plugins. */
  checkIntegration: (id: string) => request<IntegrationInfo[]>(`/api/integrations/${enc(id)}/check`, { method: 'POST' }),
  createConnection: (c: ConnectionInput) => request<Connection>('/api/connections', { method: 'POST', body: JSON.stringify(c) }),
  updateConnection: (id: string, c: Partial<ConnectionInput>) =>
    request<Connection>(`/api/connections/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(c) }),
  deleteConnection: (id: string) => request<void>(`/api/connections/${enc(id)}`, { method: 'DELETE' }),
  setConnectionSecret: (id: string, secret: string) =>
    request<Connection>(`/api/connections/${enc(id)}/secret`, { method: 'PUT', body: JSON.stringify({ secret }) }),
  /** An MCP server's tools as last listed (null before the first listing). */
  connectionTools: (id: string) => request<McpTools | null>(`/api/connections/${enc(id)}/tools`),
  /** How a connection of a kind with `status` is doing, from its plugin (null: nothing to say). */
  connectionStatus: (id: string) => request<ConnectionStatus | null>(`/api/connections/${enc(id)}/status`),
  /** Lists an MCP server's tools now. */
  refreshConnectionTools: (id: string) => request<McpTools>(`/api/connections/${enc(id)}/tools`, { method: 'POST' }),
  /** Settings → MCP servers (PDT-322): the `mcp` connections with what agents get of them. */
  mcpServers: () => request<McpServersView>('/api/mcp-servers'),
  /** Connects to an MCP server and lists its tools, resources and prompts. */
  testMcpServer: (id: string) => request<McpProbe>(`/api/mcp-servers/${enc(id)}/test`, { method: 'POST' }),
  /** Begins an MCP server's OAuth sign-in: the address the browser opens. */
  signInMcpServer: (id: string, origin: string) =>
    request<SignInStart>(`/api/mcp-servers/${enc(id)}/sign-in`, { method: 'POST', body: JSON.stringify({ origin }) }),
  signOutMcpServer: (id: string) => request<void>(`/api/mcp-servers/${enc(id)}/sign-in`, { method: 'DELETE' }),
};
