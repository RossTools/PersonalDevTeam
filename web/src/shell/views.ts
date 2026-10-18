import { Files, Search, Settings, TerminalSquare, Ticket, type LucideIcon } from 'lucide-react';

/** The views in the rail. Inbox and Personas join in Phase 7. */
export type ViewId = 'sessions' | 'tickets' | 'files' | 'settings';

/** `phone: false` leaves a view out of the phone rail (at most five items there; Files is in the palette's Go to). */
export const VIEWS: { id: ViewId; label: string; icon: LucideIcon; phone?: false }[] = [
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'tickets', label: 'Tickets', icon: Ticket },
  { id: 'files', label: 'Files', icon: Files, phone: false },
  { id: 'settings', label: 'Settings', icon: Settings },
];

/** The phone rail's Search item: opens the command palette (it is not a view). */
export const SEARCH = { label: 'Search', icon: Search };
