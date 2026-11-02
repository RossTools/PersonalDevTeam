// Test data for the Inbox: an agent's question in the structured shape `ask_user` takes (PDT-476), the Coder's
// question on PDT-401 reworked: a short question, its background, three described choices, A recommended.
import type { InboxItem } from '../api/gen/InboxItem';
import type { Question } from '../api/gen/Question';

export const structuredQuestion: Question = {
  question: 'Which approach do you want for keeping the UI token from agents?',
  context: [
    "The ticket's suggested fixes don't stop an agent. A meta tag in `index.html`, or a cookie set when the page loads:",
    "an agent's `curl 127.0.0.1:<port>/` gets the page and its `Set-Cookie` just as a browser does.",
    '',
    'Over plain HTTP, any local process can do whatever a browser can, so the browser needs a secret the agent can\'t fetch.',
  ].join('\n'),
  choices: [
    {
      label: 'A: Pair the browser',
      description: [
        'pdtd sets the UI token as an `HttpOnly SameSite=Strict` cookie only when a browser opens a one-time pairing link.',
        '- The tray\'s "Open PDT" and the startup log mint the link; an unpaired browser gets a *Pair this browser* screen.',
        '- An agent would have to read PDT\'s data folder or log to get a code: a much higher bar than adding one header.',
        '- A same-user shell can\'t be stopped completely.',
      ].join('\n'),
    },
    {
      label: 'B: Peer-process check',
      description: [
        'pdtd finds which local process owns the connection and refuses the token to anything descended from pdtd.',
        '- Nothing changes for you.',
        "- A double-forked or `nohup`'d curl escapes it, and it needs per-OS code on macOS, Linux and Windows.",
      ].join('\n'),
    },
    { label: 'C: Both', description: 'A and B together: the most work, and B still has its gaps.' },
  ],
  recommended: 'A: Pair the browser',
  why: 'It closes the hole for every agent on every OS, with one pairing per browser.',
};

/** The structured question as the Inbox lists it. */
export function structuredQuestionItem(over: Partial<InboxItem> = {}): InboxItem {
  return {
    id: 'q401', kind: 'question', group: 'question', status: 'open', projectId: 'p', ticketId: 't401', ticketKey: 'PDT-401',
    ticketTitle: "The UI token is handed to any local process that sends 'Sec-Fetch-Site: same-origin'", attemptId: 'a1',
    sessionId: 'sess', runId: 'r', stepId: 's', personaId: 'coder', personaName: 'Coder', changeRequest: null,
    title: structuredQuestion.question, body: { questions: [structuredQuestion] }, choices: [], resolution: null,
    createdAt: Date.now(), resolvedAt: null, ...over,
  };
}
