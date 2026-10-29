import type { InboxItem } from '../api/gen/InboxItem';
import { confirmOf } from './model';

/**
 * A call to one of PDT's asking tools from an agent that can't ask in its own prompt (Codex, Devin; PDT-256): what it
 * calls and with what, and what Allow and Deny do.
 */
export function ConfirmCall({ item }: { item: InboxItem }) {
  const { tool, agent, session, input } = confirmOf(item);
  return (
    <>
      <p className="text-sm">
        {session ?? 'An agent session'}{agent ? ` (${agent})` : ''} wants to call <span className="font-mono">{tool}</span>,
        which needs your yes. {agent ?? 'Its agent'} can&apos;t ask in its own prompt, so PDT asks here: Allow runs the
        call as given, Deny doesn&apos;t. Either way the agent hears what happened.
      </p>
      <pre aria-label="The call's arguments" className="max-h-96 overflow-auto rounded-md bg-muted p-2 font-mono text-xs whitespace-pre-wrap">{input}</pre>
    </>
  );
}
