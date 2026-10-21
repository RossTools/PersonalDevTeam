import { Files, Inbox, Search, Settings, TerminalSquare, Ticket, UserCog, type LucideIcon } from 'lucide-react';

/** The views in the rail. */
export type ViewId = 'sessions' | 'inbox' | 'tickets' | 'personas' | 'files' | 'settings';

/** `phone: false` leaves a view out of the phone rail (at most five items there; Files is in the palette's Go to). */
export const VIEWS: { id: ViewId; label: string; icon: LucideIcon; phone?: false }[] = [
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'inbox', label: 'Inbox', icon: Inbox },
  { id: 'tickets', label: 'Tickets', icon: Ticket },
  { id: 'personas', label: 'Personas', icon: UserCog, phone: false },
  { id: 'files', label: 'Files', icon: Files, phone: false },
  { id: 'settings', label: 'Settings', icon: Settings },
];

/** The phone rail's Search item: opens the command palette (it is not a view). */
export const SEARCH = { label: 'Search', icon: Search };
