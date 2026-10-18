import { Files, Search, Settings, TerminalSquare, type LucideIcon } from 'lucide-react';

/** The views in the rail. Tickets (backlog and board) join in Phase 4, Inbox and Personas in Phase 7. */
export type ViewId = 'sessions' | 'files' | 'settings';

export const VIEWS: { id: ViewId; label: string; icon: LucideIcon }[] = [
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'files', label: 'Files', icon: Files },
  { id: 'settings', label: 'Settings', icon: Settings },
];

/** The phone rail's Search item: opens the command palette (it is not a view). */
export const SEARCH = { label: 'Search', icon: Search };
