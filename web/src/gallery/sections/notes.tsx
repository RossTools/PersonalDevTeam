import { useState, type ReactNode } from 'react';
import type { PmNote } from '../../api/gen/PmNote';
import { Markdown } from '../../common/Markdown';
import { MarkdownEditor } from '../../common/MarkdownEditor';
import { NoteItem } from '../../notes/NoteItem';
import { SectionLabel } from '@ui/SectionLabel';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      {children}
    </div>
  );
}

const NOW = Date.now();
const note = (id: string, over: Partial<PmNote>): PmNote => ({
  id, projectId: null, kind: 'user', title: '', body: '', pinned: false, sessionId: null, createdAt: NOW, updatedAt: NOW, ...over,
});

const NOTES: { note: PmNote; projectKey: string | null }[] = [
  { note: note('a', { title: 'Ideas for the board', pinned: true, updatedAt: NOW - 2 * 60_000, body: '- **Swimlanes per area**, a compact density for small screens, and dragging a ticket onto a session to start it.' }), projectKey: null },
  { note: note('b', { title: 'Meeting with Alex', updatedAt: NOW - 26 * 3600_000, body: 'Talked about the auth rewrite. Alex prefers OIDC over our own sessions; follow up next week.' }), projectKey: null },
  { note: note('c', { title: 'Release checklist', updatedAt: NOW - 3 * 86400_000, body: '1. Bump the version\n2. Run the build-only release\n3. Smoke test on the Mac mini' }), projectKey: 'SA' },
  { note: note('d', { title: 'Books to read', updatedAt: NOW - 20 * 86400_000 }), projectKey: null },
];

const DOC = `Things I keep wanting while triaging. None of these are tickets yet.

### Layout

- **Swimlanes per area**, so the auth work and the UI work stop interleaving.
- Remember the last agent per project: \`default_agent\` is host-wide today.

> Alex: “The board is where I live; the backlog is where I plan.”

- [x] Ask Sam whether swimlanes break the phone layout
- [ ] Sketch the compact card`;

const SOURCE = `# Ideas for the board

Some **bold**, some *italic*, ~~struck~~ and \`inline code\`, with [a link](https://example.com).

## Layout

- Swimlanes per area
  - nested item
1. Numbered
2. Next

- [x] Ask Sam
- [ ] Sketch the compact card

> A quote, dimmed.

\`\`\`ts
const x: number = 1; // a comment
\`\`\`

#### Fourth level
##### Fifth level`;

function Editors() {
  const [note, setNote] = useState(SOURCE);
  const [file, setFile] = useState(SOURCE);
  return (
    <>
      <Demo label="MarkdownEditor · note (Write)">
        <article className="w-[560px] max-w-full rounded-md border border-border bg-background p-8">
          <MarkdownEditor label="Note" value={note} onChange={setNote} />
        </article>
      </Demo>
      <Demo label="MarkdownEditor · file (line numbers)">
        <div className="h-[520px] w-[560px] max-w-full overflow-hidden rounded-md border border-border">
          <MarkdownEditor label="README.md" variant="file" value={file} onChange={setFile} />
        </div>
      </Demo>
    </>
  );
}

function NoteList() {
  const [selected, setSelected] = useState('a');
  return (
    <div className="w-80 rounded-md border border-border bg-background py-1">
      <ul className="flex flex-col gap-px px-1">
        {NOTES.map(({ note: n, projectKey }) => (
          <li key={n.id}><NoteItem note={n} projectKey={projectKey} selected={n.id === selected} onOpen={setSelected} /></li>
        ))}
      </ul>
    </div>
  );
}

function render() {
  return (
    <div className="flex flex-wrap items-start gap-8">
      <Demo label="Note list items · selected, host-wide, project key, no text"><NoteList /></Demo>
      <Demo label="A note to read · text-title, Markdown variant=document">
        <article className="w-[560px] max-w-full rounded-md border border-border bg-background p-8">
          <h1 className="text-title font-semibold tracking-tight">Ideas for the board</h1>
          <p className="mt-3 mb-7 text-xs text-muted-foreground">Created 12 Oct · 64 words</p>
          <Markdown variant="document" breaks>{DOC}</Markdown>
        </article>
      </Demo>
      <Editors />
    </div>
  );
}

export const section = { title: 'Notes', order: 48, render };
