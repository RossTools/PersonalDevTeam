import type { HostChecks } from '../api/gen/HostChecks';
import type { HostSettingsView } from '../api/gen/HostSettingsView';
import type { UpdateStatus } from '../api/gen/UpdateStatus';

export interface ChecklistItem { id: string; text: string }

/** What a host still needs (spec §6 "First run"), in the order to fix it. */
export function checklist(s: HostSettingsView, u: UpdateStatus | undefined, c: HostChecks | undefined): ChecklistItem[] {
  const items: ChecklistItem[] = [];
  if (!s.ownerFromEnv && !s.settings.ownerLogin) items.push({ id: 'owner', text: 'Set the owner (your Tailscale login).' });
  if (!s.settings.remoteAccess) items.push({ id: 'remote', text: 'Remote access is off.' });
  else if (s.remote.error) items.push({ id: 'remote', text: `Remote access is not working: ${s.remote.error}` });
  if (s.tokenError) items.push({ id: 'token', text: 'The update token cannot be read from the keychain (see Host settings).' });
  else if (!s.tokenPresent) items.push({ id: 'token', text: 'No update token: this host only updates manually.' });
  else if (u?.token.state === 'expires_soon') items.push({ id: 'token', text: 'The update token expires soon; replace it.' });
  else if (u?.token.state === 'expired') items.push({ id: 'token', text: 'The update token expired or was revoked; replace it.' });
  if (c && !c.claude.ok) items.push({ id: 'claude', text: `Claude Code: ${c.claude.detail}` });
  if (c && !c.codex.ok) items.push({ id: 'codex', text: `Codex: ${c.codex.detail}` });
  return items;
}
