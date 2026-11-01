import { lazy, Suspense, type ReactNode } from 'react';
import { Skeleton } from '@ui';

// The screens and dialogs the shell loads the first time they're shown (the ticket screens and the Markdown libraries
// they use, among others): a new one is one line here, sorted by name.
export const ChangesView = lazy(() => import('../review/ChangesView').then((m) => ({ default: m.ChangesView })));
export const CreateIssueDialog = lazy(() => import('../tickets/CreateIssueDialog').then((m) => ({ default: m.CreateIssueDialog })));
export const DocumentPage = lazy(() => import('../documents/DocumentPage').then((m) => ({ default: m.DocumentPage })));
export const InboxView = lazy(() => import('../inbox/InboxView').then((m) => ({ default: m.InboxView })));
export const NewTicketDialog = lazy(() => import('../tickets/NewTicketDialog').then((m) => ({ default: m.NewTicketDialog })));
export const NewWorkflowDialog = lazy(() => import('../workflows/NewWorkflowDialog').then((m) => ({ default: m.NewWorkflowDialog })));
export const NoteScreen = lazy(() => import('../notes/NotePage').then((m) => ({ default: m.NoteScreen })));
export const PersonaPage = lazy(() => import('../personas/PersonaPage').then((m) => ({ default: m.PersonaPage })));
export const PhoneInboxItem = lazy(() => import('../inbox/PhoneInbox').then((m) => ({ default: m.PhoneInboxItem })));
export const PmView = lazy(() => import('../pm/PmView').then((m) => ({ default: m.PmView })));
export const StartDialog = lazy(() => import('../tickets/StartDialog').then((m) => ({ default: m.StartDialog })));
export const TicketView = lazy(() => import('../tickets/TicketView').then((m) => ({ default: m.TicketView })));
export const TicketsView = lazy(() => import('../tickets/TicketsView').then((m) => ({ default: m.TicketsView })));
export const WorkflowsView = lazy(() => import('../workflows/WorkflowsView').then((m) => ({ default: m.WorkflowsView })));
export const WorkspaceMergedBanner = lazy(() => import('../tickets/WorkspaceMergedBanner').then((m) => ({ default: m.WorkspaceMergedBanner })));

/** A lazy screen in the work area: a skeleton until it has loaded. */
export function ScreenSuspense({ children }: { children: ReactNode }) {
  return <Suspense fallback={<Skeleton lines={8} className="p-4" />}>{children}</Suspense>;
}
