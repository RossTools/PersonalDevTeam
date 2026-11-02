import { request } from './core';
import type { PairingStatus } from './gen/PairingStatus';
import type { PairLink } from './gen/PairLink';

/** Paired browsers: only a paired browser gets the UI token, so only it makes the changes only the user may make. */
export const pairingApi = {
  pairing: () => request<PairingStatus>('/api/ui/pairing'),
  /** PDT opens itself in this computer's default browser, paired. */
  openPaired: () => request<void>('/api/ui/open', { method: 'POST' }),
  /** A one-time link that pairs another browser. */
  newPairLink: () => request<PairLink>('/api/ui/pair-links', { method: 'POST' }, { asUser: true }),
  unpairAll: () => request<void>('/api/ui/paired', { method: 'DELETE' }, { asUser: true }),
};
