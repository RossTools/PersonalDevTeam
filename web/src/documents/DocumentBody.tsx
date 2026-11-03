// A document's content as its provider reads it, kept live (PDT-317): read again whenever PDT sees the document change
// (an agent's edit through PDT touches it), with what changed lit up for a moment. The Documents view and the panel
// beside a conversation both show it. A provider with a page of its own (its own editor, PDT-316) shows that page, or
// PDT's own reading of it (Read in PDT), which also reads again when the provider's version changes (edits made there).
import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { BarChart3 } from 'lucide-react';
import { Alert, EmptyState, SegmentedControl, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import type { CommentAnchor } from '../api/gen/CommentAnchor';
import type { DocumentContent } from '../api/gen/DocumentContent';
import type { Sheet } from '../api/gen/Sheet';
import { Markdown } from '../common/Markdown';
import { actionError } from '../common/useAction';
import { DocEditor } from './DocEditor';
import { documentKeys, useDocument, useDocumentContent, useDocumentProviders, useDocumentVersion } from './queries';

// IronCalc's editor (and its WASM) loads only when a local sheet opens (PDT-315).
const SheetEditor = lazy(() => import('./SheetEditor'));

/** What a live sheet tells the page: its selection (comments go on it), how to show a range, and the sheet on show. */
export type SheetHandle = { selection: () => CommentAnchor | null; show: (a: CommentAnchor) => void; sheet: () => string };

/** How long a change stays lit. */
const HIGHLIGHT_MS = 2500;

const letter = (i: number): string => (i < 26 ? String.fromCharCode(65 + i) : letter(Math.floor(i / 26) - 1) + letter(i % 26));

/** The cells (`Sheet1!B3`) whose value or formula differs between two reads of a spreadsheet. */
export function changedCells(before: Sheet[], after: Sheet[]): Set<string> {
  const out = new Set<string>();
  for (const s of after) {
    const old = before.find((b) => b.name === s.name);
    s.rows.forEach((row, r) => row.forEach((cell, c) => {
      const was = old?.rows[r]?.[c];
      if ((was?.value ?? '') !== cell.value || (was?.formula ?? null) !== (cell.formula ?? null)) out.add(`${s.name}!${letter(c)}${r + 1}`);
    }));
    for (const ch of s.charts) {
      const was = old?.charts.find((o) => o.id === ch.id);
      if (!was || JSON.stringify(was.spec) !== JSON.stringify(ch.spec)) out.add(`${s.name}#${ch.id}`);
    }
  }
  return out;
}

/** A sheet as a table: its rows from A1, with column letters and row numbers; a cell's formula shows on hover. */
function SheetTable({ sheet, changed }: { sheet: Sheet; changed: Set<string> }) {
  const width = Math.max(0, ...sheet.rows.map((r) => r.length));
  if (sheet.rows.length === 0 && sheet.charts.length === 0) return <p className="text-xs text-muted-foreground">This sheet is empty.</p>;
  return (
    <div className="flex flex-col gap-2">
      {sheet.rows.length > 0 && (
        <div className="overflow-auto rounded-md border border-border">
          <table className="border-collapse text-xs">
            <thead>
              <tr>
                <th className="sticky left-0 bg-muted px-2 py-1" />
                {Array.from({ length: width }, (_, i) => <th key={i} scope="col" className="border-l border-border bg-muted px-2 py-1 font-medium text-muted-foreground">{letter(i)}</th>)}
              </tr>
            </thead>
            <tbody>
              {sheet.rows.map((row, r) => (
                <tr key={r} className="border-t border-border">
                  <th scope="row" className="sticky left-0 bg-muted px-2 py-1 text-right font-medium text-muted-foreground">{r + 1}</th>
                  {Array.from({ length: width }, (_, c) => {
                    const cell = row[c];
                    const lit = changed.has(`${sheet.name}!${letter(c)}${r + 1}`);
                    return (
                      <td key={c} data-changed={lit || undefined} className={cn('border-l border-border px-2 py-1 whitespace-nowrap transition-colors duration-700', lit && 'bg-accent')} title={cell?.formula ?? undefined}>
                        {cell?.value ?? ''}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {sheet.charts.length > 0 && (
        <ul aria-label="Charts" className="flex flex-col gap-1">
          {sheet.charts.map((ch) => {
            const lit = changed.has(`${sheet.name}#${ch.id}`);
            return (
              <li key={ch.id} data-changed={lit || undefined} className={cn('flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-xs transition-colors duration-700', lit && 'bg-accent')}>
                <BarChart3 aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{ch.spec.title ?? 'Chart'}</span>
                <span className="shrink-0 text-meta text-muted-foreground">{ch.spec.kind} · {ch.spec.data}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/**
 * A doc's Markdown, with the blocks that weren't there in the last read lit for a moment. The blocks are compared by
 * their text after rendering, so a list or a table lights as a whole.
 */
function LiveMarkdown({ markdown }: { markdown: string }) {
  const box = useRef<HTMLDivElement>(null);
  const seen = useRef<Set<string> | null>(null);
  useLayoutEffect(() => {
    const root = box.current?.firstElementChild;
    if (!root) return;
    const blocks = Array.from(root.children) as HTMLElement[];
    const texts = blocks.map((b) => b.textContent ?? '');
    const before = seen.current;
    seen.current = new Set(texts);
    if (!before) return;
    const lit = blocks.filter((_, i) => !before.has(texts[i]!));
    for (const b of lit) b.dataset.changed = 'true';
    if (lit.length === 0) return;
    const t = window.setTimeout(() => lit.forEach((b) => delete b.dataset.changed), HIGHLIGHT_MS);
    return () => window.clearTimeout(t);
  }, [markdown]);
  if (!markdown.trim()) return <p className="text-xs text-muted-foreground">This document is empty.</p>;
  return (
    <div ref={box} className="[&_[data-changed]]:bg-accent [&>div>*]:rounded-sm [&>div>*]:transition-colors [&>div>*]:duration-700">
      <Markdown>{markdown}</Markdown>
    </div>
  );
}

function Content({ content, changed }: { content: DocumentContent; changed: Set<string> }) {
  const [sheet, setSheet] = useState(0);
  if (content.kind === 'doc') return <LiveMarkdown markdown={content.markdown} />;
  const at = Math.min(sheet, content.sheets.length - 1);
  const shown = content.sheets[at];
  return (
    <div className="flex flex-col gap-2">
      {content.sheets.length > 1 && (
        <SegmentedControl
          aria-label="Sheet"
          size="sm"
          className="self-start"
          options={content.sheets.map((s, i) => ({ value: String(i), label: s.name }))}
          value={String(at)}
          onChange={(v) => setSheet(Number(v))}
        />
      )}
      {shown ? <SheetTable sheet={shown} changed={changed} /> : <EmptyState size="inline">No sheets.</EmptyState>}
    </div>
  );
}

/** What changed in a spreadsheet's last read, lit for a moment (none on the first read). */
function useChangedCells(content: DocumentContent | undefined): Set<string> {
  const last = useRef<Sheet[] | null>(null);
  const [changed, setChanged] = useState<Set<string>>(() => new Set());
  useEffect(() => {
    if (content?.kind !== 'sheet') return;
    const before = last.current;
    last.current = content.sheets;
    if (!before) return;
    const c = changedCells(before, content.sheets);
    if (c.size === 0) return;
    setChanged(c);
    const t = window.setTimeout(() => setChanged(new Set()), HIGHLIGHT_MS);
    return () => window.clearTimeout(t);
  }, [content]);
  return changed;
}

type View = 'live' | 'pdt';
/**
 * What a provider's live editor may do in its frame (PDT-421): run as its own site, open its links and dialogs in new
 * windows, and download, but never navigate PDT's own page away.
 */
const EMBED_SANDBOX = 'allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads';

const VIEW_KEY = 'pdt.documents.view';

/**
 * A document's content, live: read again each time PDT sees it change. A provider that edits live in PDT (its local
 * one) shows its editors instead: a doc's Rendered/Write editor, and a sheet's spreadsheet editor (`onSheet` gets its
 * handle).
 */
export function DocumentBody({ id, providerLabel, className, onSheet }: {
  id: string;
  providerLabel: string;
  className?: string;
  onSheet?: (h: SheetHandle | null) => void;
}) {
  const qc = useQueryClient();
  const doc = useDocument(id);
  const content = useDocumentContent(id);
  const providers = useDocumentProviders();
  const provider = providers.data?.providers.find((p) => p.id === doc.data?.provider);
  const editsHere = !!provider?.capabilities.live;
  const updatedAt = doc.data?.updatedAt;
  const read = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (updatedAt === undefined) return;
    if (read.current !== undefined && read.current !== updatedAt) void qc.invalidateQueries({ queryKey: documentKeys.content(id) });
    read.current = updatedAt;
  }, [qc, id, updatedAt]);
  const changed = useChangedCells(content.data?.content);
  const embed = content.data?.embedUrl ?? null;
  const [view, setView] = useState<View>(() => (localStorage.getItem(VIEW_KEY) === 'pdt' ? 'pdt' : 'live'));
  const live = embed !== null && view === 'live';
  // Edits made at the provider (in its own editor) don't pass through PDT: its version says when to read again. The
  // embedded editor follows them itself, and a provider without one is only changed through PDT (`updatedAt`).
  const version = useDocumentVersion(id, content.isSuccess && embed !== null && view === 'pdt');
  const shownVersion = content.data?.content.version ?? null;
  const nowVersion = version.data?.version ?? null;
  useEffect(() => {
    if (nowVersion !== null && shownVersion !== null && nowVersion !== shownVersion) {
      void qc.invalidateQueries({ queryKey: documentKeys.content(id) });
    }
  }, [qc, id, nowVersion, shownVersion]);
  const pickView = (v: View) => {
    localStorage.setItem(VIEW_KEY, v);
    setView(v);
  };

  if (content.isPending) return <Skeleton lines={6} label="Reading the document" className={className} />;
  if (content.isError) return <Alert tone="error" className={className}>{providerLabel} could not read it: {actionError(content.error)}</Alert>;
  const c = content.data.content;
  if (editsHere && c.kind === 'doc') {
    return <div className={className}><DocEditor id={id} markdown={c.markdown} version={c.version ?? null} editable={!!provider?.capabilities.edit} /></div>;
  }
  if (editsHere && c.kind === 'sheet') {
    return (
      <div className={className}>
        <Suspense fallback={<Skeleton lines={8} label="Opening the sheet" />}>
          <SheetEditor id={id} onReady={onSheet} />
        </Suspense>
      </div>
    );
  }
  const body = <Content content={c} changed={changed} />;
  if (embed === null) return <div className={className}>{body}</div>;
  return (
    <div className={cn('flex flex-col gap-2', className)}>
      <SegmentedControl
        aria-label="How to show it"
        size="sm"
        className="self-start"
        options={[{ value: 'live', label: `${providerLabel} editor` }, { value: 'pdt', label: 'Read in PDT' }]}
        value={view}
        onChange={(v) => pickView(v as View)}
      />
      {live ? (
        <>
          <iframe title={`${doc.data?.title ?? 'Document'} in ${providerLabel}`} src={embed} allow="clipboard-read; clipboard-write" sandbox={EMBED_SANDBOX} className="min-h-96 w-full flex-1 rounded-md border border-border" />
          <p className="text-xs text-muted-foreground">
            Asks you to sign in, or stays blank? This browser keeps {providerLabel}’s sign-in out of pages inside PDT: switch to Read in PDT, or open it in {providerLabel}.
          </p>
        </>
      ) : body}
    </div>
  );
}
