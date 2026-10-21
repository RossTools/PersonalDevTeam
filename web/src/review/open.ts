import { createContext, useContext } from 'react';

/** `here`: show the changes in the workspace on screen, after the current tab (moving the tab there if another
 * workspace has it). `ticketId`: in the workspace that has that ticket's tab, right after it (moving the tab there if
 * another workspace has it), e.g. from the Inbox. Otherwise the tab is shown wherever it is, or opens in its project's
 * workspace. */
export interface OpenChangesOptions { here?: boolean; ticketId?: string }

/** Opens (or focuses) the changes tab of a worktree. AppShell provides it. */
export const OpenChangesContext = createContext<(w: { id: string; projectId: string }, opts?: OpenChangesOptions) => void>(() => {});
export const useOpenChanges = () => useContext(OpenChangesContext);
