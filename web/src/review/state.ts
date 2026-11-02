/** The view-state names of a worktree's Changes (PDT-379): what the page comes back to when it is left. */
export const changesState = (worktreeId: string, what: string) => `changes:${worktreeId}:${what}`;
