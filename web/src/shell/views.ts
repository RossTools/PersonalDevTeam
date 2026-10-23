import { BriefcaseBusiness, Files, Inbox, Settings, TerminalSquare, Ticket, UserCog, Workflow, type LucideIcon } from 'lucide-react';

/** The views in the rail. */
export type ViewId = 'sessions' | 'inbox' | 'pm' | 'tickets' | 'workflows' | 'personas' | 'files' | 'settings';

/** The views, in order: the desktop rail and the phone drawer (PDT-112) both list them. */
export const VIEWS: { id: ViewId; label: string; icon: LucideIcon }[] = [
  { id: 'pm', label: 'Project manager', icon: BriefcaseBusiness },
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'inbox', label: 'Inbox', icon: Inbox },
  { id: 'tickets', label: 'Tickets', icon: Ticket },
  { id: 'workflows', label: 'Workflows', icon: Workflow },
  { id: 'personas', label: 'Personas', icon: UserCog },
  { id: 'files', label: 'Files', icon: Files },
  { id: 'settings', label: 'Settings', icon: Settings },
];
