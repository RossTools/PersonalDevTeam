import { Alert, Button } from '@ui';
import { navigate } from '../shell/router';
import { useAgentAvailability } from './useAgentAvailability';
import { useAgents } from './agents';

/** Whether the host checks say no agent is installed and switched on here; false until they load. */
export function useNoAgent(): boolean {
  const availability = useAgentAvailability();
  const agents = useAgents();
  return !agents.all.some(availability.offered);
}

/**
 * Said where an agent would start when none can (PDT-718): no agent is installed (or the server refused the one
 * asked for, `message`), and where to fix it, with Open Get started.
 */
export function NoAgentNotice({ message }: { message?: string }) {
  return (
    <Alert
      tone="warning"
      title={message ? 'That agent isn\'t available' : 'No agent is installed yet'}
      action={<Button size="sm" variant="secondary" onClick={() => navigate('/setup')}>Open Get started</Button>}
    >
      {message ?? 'Install Claude Code, Codex or Devin (or set its path) under Agents on this machine in Get started.'}
    </Alert>
  );
}
