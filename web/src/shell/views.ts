import { BriefcaseBusiness, FileText, Files, Inbox, NotebookPen, Settings, TerminalSquare, Ticket, UserCog, Workflow, type LucideIcon } from 'lucide-react';

/** The views in the rail. */
export type ViewId = 'sessions' | 'inbox' | 'pm' | 'tickets' | 'notes' | 'documents' | 'workflows' | 'personas' | 'files' | 'settings';

/** The views, in order: the desktop rail and the phone drawer (PDT-112) both list them. */
/** `dot`: what a dot on the view's item means, for its accessible name. */
export const VIEWS: { id: ViewId; label: string; icon: LucideIcon; dot?: string }[] = [
  { id: 'pm', label: 'Project manager', icon: BriefcaseBusiness, dot: 'new reply' },
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'inbox', label: 'Inbox', icon: Inbox, dot: 'new updates' },
  { id: 'tickets', label: 'Tickets', icon: Ticket },
  { id: 'notes', label: 'Notes', icon: NotebookPen },
  { id: 'documents', label: 'Documents', icon: FileText },
  { id: 'workflows', label: 'Workflows', icon: Workflow },
  { id: 'personas', label: 'Personas', icon: UserCog },
  { id: 'files', label: 'Files', icon: Files },
  { id: 'settings', label: 'Settings', icon: Settings },
];
