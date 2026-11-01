import { enc, queryString, request, requestText } from './core';
import type { BugReportFiled } from './gen/BugReportFiled';
import type { BugReportPreview } from './gen/BugReportPreview';
import type { CheckResponse } from './gen/CheckResponse';
import type { FolderEntry } from './gen/FolderEntry';
import type { FolderListing } from './gen/FolderListing';
import type { GuideInfo } from './gen/GuideInfo';
import type { Health } from './gen/Health';
import type { HostChecks } from './gen/HostChecks';
import type { HostSettingsPatch } from './gen/HostSettingsPatch';
import type { HostSettingsView } from './gen/HostSettingsView';
import type { LogEvent } from './gen/LogEvent';
import type { NetworkSettings } from './gen/NetworkSettings';
import type { NewBugReport } from './gen/NewBugReport';
import type { NewFolder } from './gen/NewFolder';
import type { RemoteStatus } from './gen/RemoteStatus';
import type { SetupPatch } from './gen/SetupPatch';
import type { SetupView } from './gen/SetupView';
import type { UpdateStatus } from './gen/UpdateStatus';
import type { WhoAmI } from './gen/WhoAmI';

/** This PDT: health, host folders, settings, updates, checks, setup, guides, bug reports and the event log. */
export const hostApi = {
  health: () => request<Health>('/api/health'),
  listFolders: (q: { path?: string; hidden?: boolean; nearest?: boolean }) =>
    request<FolderListing>(`/api/host/folders${queryString({ ...q, hidden: q.hidden || undefined, nearest: q.nearest || undefined })}`),
  createFolder: (f: NewFolder) => request<FolderEntry>('/api/host/folders', { method: 'POST', body: JSON.stringify(f) }),
  listLogEvents: (q: { type?: string; limit?: number; after?: number } = {}) => request<LogEvent[]>(`/api/events${queryString(q)}`),
  listGuides: () => request<GuideInfo[]>('/api/guides'),
  /** A built-in guide or prompt, as Markdown. */
  guide: (name: string) => requestText(`/api/guides/${enc(name)}`),
  hostSettings: () => request<HostSettingsView>('/api/host/settings'),
  saveHostSettings: (p: HostSettingsPatch) =>
    request<HostSettingsView>('/api/host/settings', { method: 'PUT', body: JSON.stringify(p) }),
  /** Installs (or updates) a built-in ACP profile's program from the ACP registry (PDT-239). */
  installAcpAgent: (id: string) => request<HostSettingsView>(`/api/host/acp-agents/${enc(id)}/install`, { method: 'POST' }),
  /** Asks a built-in ACP profile's server to sign in: it opens the browser on PDT's computer; this answers when that is done. */
  signInAcpAgent: (id: string) => request<HostSettingsView>(`/api/host/acp-agents/${enc(id)}/sign-in`, { method: 'POST' }),
  /** Settings → Network (PDT-231): saved now, applied when PDT restarts. */
  saveNetwork: (n: NetworkSettings) =>
    request<HostSettingsView>('/api/host/network', { method: 'PUT', body: JSON.stringify(n) }),
  /** Restart now: pdtd exits and its host supervisor starts it again (409 `not_supervised` without one). */
  restartHost: () => request<void>('/api/host/restart', { method: 'POST' }),
  refreshRemote: () => request<RemoteStatus>('/api/host/remote/refresh', { method: 'POST' }),
  whoami: () => request<WhoAmI>('/api/host/whoami'),
  updateStatus: () => request<UpdateStatus>('/api/host/update'),
  checkUpdate: () => request<CheckResponse>('/api/host/update/check', { method: 'POST' }),
  installUpdate: (force: boolean) =>
    request<UpdateStatus>('/api/host/update/install', { method: 'POST', body: JSON.stringify({ force }) }),
  dismissUpdateResult: () => request<void>('/api/host/update/dismiss', { method: 'POST' }),
  saveToken: (token: string) => request<UpdateStatus>('/api/host/token', { method: 'PUT', body: JSON.stringify({ token }) }),
  deleteToken: () => request<void>('/api/host/token', { method: 'DELETE' }),
  hostChecks: () => request<HostChecks>('/api/host/checks'),
  /** Get started (PDT-230): the first-run checklist's steps, and whether the app opens on it. */
  hostSetup: () => request<SetupView>('/api/host/setup'),
  saveHostSetup: (p: SetupPatch) => request<SetupView>('/api/host/setup', { method: 'PUT', body: JSON.stringify(p) }),
  /** Where a bug report would go, and the redacted diagnostics it would carry (PDT-153). */
  bugReportPreview: () => request<BugReportPreview>('/api/bug-reports/preview'),
  reportBug: (r: NewBugReport) =>
    request<BugReportFiled>('/api/bug-reports', { method: 'POST', body: JSON.stringify(r) }),
  refreshChecks: () => request<HostChecks>('/api/host/checks/refresh', { method: 'POST' }),
};
