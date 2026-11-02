import type { TicketAgent } from '../api/gen/TicketAgent';
import type { AgentStateValue } from './AgentState';

/** The card's line about who owns its workflow step now and in what state (Phase 7c). */
export function agentLine(a: TicketAgent): { text: string; dot: AgentStateValue } {
  const dot = (kind: AgentStateValue['dot'], pulse = false): AgentStateValue =>
    ({ dot: kind, pulse, label: null, tone: kind === 'running' ? 'running' : kind === 'waiting' ? 'waiting' : kind === 'failed' ? 'failed' : 'muted', unseen: false });
  const who = a.persona ?? 'Agent';
  switch (a.state) {
    case 'working': return { text: `${who} · working`, dot: dot('running', a.busy) };
    case 'asking': return { text: `${who} asks you`, dot: dot('waiting') };
    case 'stalled': return { text: `${who} stalled`, dot: dot('failed') };
    case 'yours': return { text: a.persona ? `Your turn · ${a.persona}` : 'Your turn', dot: dot('waiting') };
    case 'queued': return { text: `${who} · waiting for a slot`, dot: dot('idle') };
    case 'background': return {
      text: a.background.length ? `${who} · waiting on background task: ${a.background.join(', ')}` : `${who} · waiting on background tasks`,
      dot: dot('running'),
    };
  }
}
