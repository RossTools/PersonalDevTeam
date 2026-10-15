import { Bot, Files, Inbox, KanbanSquare, TerminalSquare, type LucideIcon } from 'lucide-react';

export type ViewId = 'inbox' | 'board' | 'sessions' | 'personas' | 'files';

export const VIEWS: { id: ViewId; label: string; icon: LucideIcon }[] = [
  { id: 'inbox', label: 'Inbox', icon: Inbox },
  { id: 'board', label: 'Board', icon: KanbanSquare },
  { id: 'sessions', label: 'Sessions', icon: TerminalSquare },
  { id: 'personas', label: 'Personas', icon: Bot },
  { id: 'files', label: 'Files', icon: Files },
];
