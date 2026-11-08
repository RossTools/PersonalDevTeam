import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useQueryClient } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { AGENT_KINDS, availabilityOf, useAgentAvailability } from './useAgentAvailability';

afterEach(() => vi.unstubAllGlobals());

const tool = (found: boolean, ok: boolean) => ({ found, ok, detail: '', path: null });
const checks = (devinFound: boolean) =>
  ({ claude: tool(true, true), codex: tool(true, false), devin: tool(devinFound, false), git: tool(true, true), gitIdentity: tool(true, true), tailscale: tool(true, true), acp: {}, checkedAt: 1 });

/** Lists the offered agents; the button does what Host settings' "Run checks again" does. */
function Probe() {
  const qc = useQueryClient();
  const a = useAgentAvailability();
  return (
    <>
      <ul>{AGENT_KINDS.filter(a.offered).map((k) => <li key={k}>{k}{a.hint(k) ? ` (${a.hint(k)})` : ''}</li>)}</ul>
      <button type="button" onClick={() => void api.refreshChecks().then((c) => qc.setQueryData(['host-checks'], c))}>Run checks again</button>
    </>
  );
}

describe('agent availability', () => {
  it('is ready until the checks load, missing when not found, logged out when found but not ok', () => {
    expect(availabilityOf(undefined, 'devin')).toBe('ready');
    expect(availabilityOf(checks(false), 'devin')).toBe('missing');
    expect(availabilityOf(checks(true), 'devin')).toBe('logged_out');
    expect(availabilityOf(checks(true), 'claude')).toBe('ready');
    // An ACP agent's check is under `acp`; one without a check (just added) is offered.
    const acp = { ...checks(true), acp: { 'acp:fake': { found: false, ok: false, detail: '', path: null } } };
    expect(availabilityOf(acp, 'acp:fake')).toBe('missing');
    expect(availabilityOf(acp, 'acp:new')).toBe('ready');
    // Switched off in Settings → Agents (PDT-270), found or not.
    expect(availabilityOf(checks(true), 'claude', ['claude'])).toBe('off');
    expect(availabilityOf(undefined, 'acp:new', ['acp:new'])).toBe('off');
  });

  it('offers every agent when the checks fail', async () => {
    stubApi({ 'GET /api/host/checks': new Reply(500, { code: 'x', message: 'x' }) });
    const { qc } = renderWithQuery(<Probe />);
    // Every agent is offered while the checks load too: wait for them to have failed.
    await waitFor(() => expect(qc.getQueryState(['host-checks'])?.status).toBe('error'));
    expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual(['claude', 'codex', 'devin']);
  });

  it('follows a refresh of the checks (a newly installed agent appears)', async () => {
    stubApi({ 'GET /api/host/checks': checks(false), 'POST /api/host/checks/refresh': checks(true) });
    renderWithQuery(<Probe />);
    await waitFor(() => expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual(['claude', 'codex (not logged in)']));
    await userEvent.click(screen.getByRole('button', { name: 'Run checks again' }));
    await waitFor(() => expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual(['claude', 'codex (not logged in)', 'devin (not logged in)']));
  });
});
