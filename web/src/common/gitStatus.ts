// The colour of a git status letter (A, M, D, R), the same in the review's file list and the file browser.

/** A file's git status letter's text colour: added green, modified amber, deleted red, renamed blue. */
export const GIT_STATUS_TONE: Record<'A' | 'M' | 'D' | 'R', string> = {
  A: 'text-running', M: 'text-waiting', D: 'text-failed', R: 'text-link',
};
