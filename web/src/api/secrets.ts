import { enc, request } from './core';
import type { NewSecret } from './gen/NewSecret';
import type { Secret } from './gen/Secret';
import type { SecretPatch } from './gen/SecretPatch';
import type { SecretStoreInfo } from './gen/SecretStoreInfo';
import type { SecretUse } from './gen/SecretUse';

/** Secrets and where they are used. */
export const secretsApi = {
  listSecrets: () => request<Secret[]>('/api/secrets'),
  secretStore: () => request<SecretStoreInfo>('/api/secrets/store'),
  createSecret: (s: NewSecret) => request<Secret>('/api/secrets', { method: 'POST', body: JSON.stringify(s) }),
  patchSecret: (id: string, p: SecretPatch) =>
    request<Secret>(`/api/secrets/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
  deleteSecret: (id: string) => request<void>(`/api/secrets/${enc(id)}`, { method: 'DELETE' }),
  secretUses: (id: string) => request<SecretUse[]>(`/api/secrets/${enc(id)}/uses`),
};
