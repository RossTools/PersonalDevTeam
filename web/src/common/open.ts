import { createContext, useContext } from 'react';
import { routePath, type SettingsPage } from '../shell/route';
import { navigate } from '../shell/router';

/** Opens (or focuses) a ticket's tab in its project's workspace. AppShell provides it. */
export const OpenTicketContext = createContext<(t: { id: string; projectId: string }) => void>(() => {});
export const useOpenTicket = () => useContext(OpenTicketContext);

/** Shows a session's tab (opening one in its project's workspace when it has none). AppShell provides it. */
export const OpenSessionContext = createContext<(s: { id: string; projectId: string }) => void>(() => {});
export const useOpenSession = () => useContext(OpenSessionContext);

/** `here`: show the changes in the workspace on screen, after the current tab (moving the tab there if another
 * workspace has it). `ticketId`: in the workspace that has that ticket's tab, right after it (moving the tab there if
 * another workspace has it), e.g. from the Inbox. Otherwise the tab is shown wherever it is, or opens in its project's
 * workspace. */
export interface OpenChangesOptions { here?: boolean; ticketId?: string }

/** Opens (or focuses) the changes tab of a worktree. AppShell provides it. */
export const OpenChangesContext = createContext<(w: { id: string; projectId: string }, opts?: OpenChangesOptions) => void>(() => {});
export const useOpenChanges = () => useContext(OpenChangesContext);

/** Shows a workflow in the Workflows view. */
export const openWorkflow = (id: string) => navigate(routePath({ view: 'workflows', target: { kind: 'workflow', id } }));

/** Shows a Settings page. */
export const openSettings = (page: SettingsPage) => navigate(routePath({ view: 'settings', page }));

/** Shows a plugin's page in Settings → Plugins. */
export const openPlugin = (id: string) => openSettings({ kind: 'plugins', id });

/** Opens a URL outside PDT, in a new browser tab that can't reach back into PDT. */
export const openExternal = (url: string) => void window.open(url, '_blank', 'noopener,noreferrer');
