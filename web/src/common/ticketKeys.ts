import { createContext } from 'react';

/**
 * The project keys whose ticket keys (`PDT-12`) Markdown turns into links to the ticket (`/tickets/PDT-12`, PDT-111).
 * The app provides its projects' keys; none (the default) links nothing. Apart from Markdown.tsx, so the shell can
 * provide it without loading the Markdown libraries.
 */
export const TicketKeysContext = createContext<readonly string[]>([]);
