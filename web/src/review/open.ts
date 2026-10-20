import { createContext, useContext } from 'react';

/** `here`: show the changes in the workspace on screen, after the current tab (moving the tab there if another
 * workspace has it). Otherwise the tab is shown wherever it is, or opens in its project's workspace. */
export interface OpenChangesOptions { here?: boolean }

/** Opens (or focuses) the changes tab of a worktree. AppShell provides it. */
export const OpenChangesContext = createContext<(w: { id: string; projectId: string }, opts?: OpenChangesOptions) => void>(() => {});
export const useOpenChanges = () => useContext(OpenChangesContext);
