// The Inbox's query keys. Everything sits under `all`, so a write or a pushed event refreshes the list and the
// answers together; each query has a key of its own, so a write to one never lands in the other's cache.
export const inboxKeys = {
  all: ['inbox'] as const,
  /** The items waiting on the user. */
  list: ['inbox', 'list'] as const,
  /** The last answers given (Recently answered), from the event log. */
  answered: ['inbox', 'answered'] as const,
};
