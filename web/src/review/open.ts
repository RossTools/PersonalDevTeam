import { createContext, useContext } from 'react';

/** Opens (or focuses) the changes tab of a worktree. AppShell provides it. */
export const OpenChangesContext = createContext<(w: { id: string; projectId: string }) => void>(() => {});
export const useOpenChanges = () => useContext(OpenChangesContext);
