import type { ReactNode } from 'react';
import { Link } from '@ui';
import type { BuiltinAgent } from './agents';
import { useHostSettings } from '../host/queries';

/** How to log in to each built-in agent, in a terminal (a Conversation session can't). */
const LOGIN: Record<BuiltinAgent, ReactNode> = {
  claude: <>run <code className="font-mono">claude</code> in a terminal and type <code className="font-mono">/login</code></>,
  codex: <>run <code className="font-mono">codex login</code> in a terminal</>,
  devin: <>run <code className="font-mono">devin auth login</code> in a terminal</>,
};

/** The API key the Subscription credentials setting removes from each agent's environment (Devin has none). */
const API_KEY: Partial<Record<BuiltinAgent, string>> = { claude: 'ANTHROPIC_API_KEY', codex: 'OPENAI_API_KEY' };

/**
 * What to do about an installed agent that isn't logged in (PDT-556): how to log in, and, while the host's credentials
 * are Subscription, that an API key alone doesn't count. `onAgentSettings` makes "Settings › This host › Agents" a link.
 */
export function LoginHint({ agent, onAgentSettings }: { agent: BuiltinAgent; onAgentSettings?: () => void }) {
  const creds = useHostSettings().data?.settings.agentCredentials;
  const key = API_KEY[agent];
  const where = onAgentSettings ? (
    <Link onClick={onAgentSettings}>Settings › This host › Agents</Link>
  ) : 'Settings › This host › Agents';
  return (
    <>
      To log in, {LOGIN[agent]}, then check again.
      {key && creds === 'subscription' && (
        <> Using an API key instead? Credentials is set to Subscription, which keeps <code className="font-mono">{key}</code> from agents: set it to Pass through in {where}.</>
      )}
    </>
  );
}
