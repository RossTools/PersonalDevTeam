import { createContext, useContext } from 'react';
import type { TabPlacement } from '../api/gen/TabPlacement';

/** Where a ticket's other tabs (its changes, a run of its branch) go when opened away from it, e.g. from the Inbox. */
export interface TicketTabs {
  /** Right after the ticket's tab, in the workspace that has it (its Start workspace); undefined when no workspace
   * has it, so the server's default applies (the end of the project's workspace). */
  placementNear: (ticketId: string) => TabPlacement | undefined;
  /** Shows a run's tab (switching to the workspaces view). */
  openRun: (r: { id: string; projectId: string }) => void;
}

/** AppShell provides it. */
export const TicketTabsContext = createContext<TicketTabs>({ placementNear: () => undefined, openRun: () => {} });
export const useTicketTabs = () => useContext(TicketTabsContext);
