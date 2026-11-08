import { useState, type ReactNode } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { CaseSensitive, Regex, Search, WholeWord, X } from 'lucide-react';
import { Alert, Badge, Disclosure, IconButton, Input, Spinner } from '@ui';
import { api } from '../api/client';
import type { FileSearchQuery } from '../api/files';
import type { SearchFile } from '../api/gen/SearchFile';
import type { SearchLine } from '../api/gen/SearchLine';
import type { Place } from '../common/PlacePicker';
import { useDebounced } from '../common/useDebounced';
import type { FolderViewState } from './useFolderView';

/**
 * Search inside the checkout's files (`git grep`: ignored and binary files are left out). With nothing typed it shows
 * `children` (the file tree); with a search, the matches grouped by file, each opening the file at its line.
 */
export function FileSearch({ state, place, children }: { state: FolderViewState; place: Place; children: ReactNode }) {
  const [s, setS] = useState<FileSearchQuery>({ q: '', case: false, word: false, regex: false, glob: '' });
  const set = (patch: Partial<FileSearchQuery>) => setS((was) => ({ ...was, ...patch }));
  const asked = useDebounced(s, 250);
  const searching = asked.q !== '';
  // A new search cancels the one in flight: the query's signal aborts once its key is left.
  const r = useQuery({
    queryKey: ['fileSearch', place.projectId, place.worktreeId, asked],
    queryFn: ({ signal }) => api.searchFiles(place.projectId, place.worktreeId, asked, signal),
    enabled: searching,
    placeholderData: keepPreviousData,
    staleTime: 10_000,
    retry: false,
  });
  return (
    <div className="flex min-h-0 flex-col gap-1.5">
      <div className="flex flex-col gap-1 px-1">
        <div className="relative flex items-center">
          <Search aria-hidden className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground" />
          <Input size="sm" type="search" aria-label="Search in files" placeholder="Search in files" value={s.q}
            className="pl-7 pr-24 [&::-webkit-search-cancel-button]:hidden"
            onChange={(e) => set({ q: e.target.value })}
            onKeyDown={(e) => { if (e.key === 'Escape' && s.q) { e.preventDefault(); set({ q: '' }); } }} />
          <div className="absolute right-0.5 flex items-center">
            {s.q && <IconButton label="Clear the search" size="sm" onClick={() => set({ q: '' })}><X aria-hidden /></IconButton>}
            <IconButton label="Match case" size="sm" pressed={s.case} onClick={() => set({ case: !s.case })}><CaseSensitive aria-hidden /></IconButton>
            <IconButton label="Whole word" size="sm" pressed={s.word} onClick={() => set({ word: !s.word })}><WholeWord aria-hidden /></IconButton>
            <IconButton label="Regular expression" size="sm" pressed={s.regex} onClick={() => set({ regex: !s.regex })}><Regex aria-hidden /></IconButton>
          </div>
        </div>
        {(s.q || s.glob) && (
          <Input size="sm" mono aria-label="Files to search" placeholder="Files to search, e.g. src/**/*.rs, !*.test.ts" value={s.glob}
            onChange={(e) => set({ glob: e.target.value })} />
        )}
      </div>
      {!s.q ? children : (
        <div className="flex min-h-0 flex-col gap-1">
          {!searching || (r.isFetching && !r.data && !r.isError) ? (
            <p className="flex items-center gap-1.5 px-2 text-xs text-muted-foreground"><Spinner decorative /> Searching…</p>
          ) : r.isError ? (
            <div className="px-1"><Alert tone="error">{r.error.message}</Alert></div>
          ) : r.data ? (
            <Results files={r.data.files} capped={r.data.capped} timedOut={r.data.timedOut} busy={r.isFetching} state={state} />
          ) : null}
        </div>
      )}
    </div>
  );
}

function Results({ files, capped, timedOut, busy, state }: { files: SearchFile[]; capped: boolean; timedOut: boolean; busy: boolean; state: FolderViewState }) {
  if (files.length === 0 && !timedOut) return <p className="px-2 text-xs text-muted-foreground">{busy ? 'Searching…' : 'No results.'}</p>;
  const shown = files.reduce((n, f) => n + f.lines.length, 0);
  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  return (
    <>
      <p role="status" className="flex items-center gap-1.5 px-2 text-xs text-muted-foreground">
        {busy && <Spinner decorative />}
        {timedOut
          ? `The search took too long: showing the ${plural(shown, 'match', 'matches')} found by then.`
          : capped
            ? `Showing the first ${plural(shown, 'match', 'matches')}, in ${plural(files.length, 'file')}.`
            : `${plural(shown, 'match', 'matches')} in ${plural(files.length, 'file')}.`}
      </p>
      <div className="flex flex-col">
        {files.map((f) => (
          <Disclosure key={f.path} variant="row" defaultOpen aria-label={f.path} triggerClassName="px-1"
            title={<span className="block truncate font-mono text-xs [direction:rtl]" title={f.path}><bdi>{f.path}</bdi></span>}
            trailing={<Badge size="sm" count={f.count} max={9999} />}>
            <ul className="flex flex-col pb-1">
              {f.lines.map((l) => (
                <li key={l.line}>
                  <MatchRow line={l} onOpen={() => state.openAt(f.path, l.line, l.offset + (l.ranges[0]?.[0] ?? 0), l.offset + (l.ranges[0]?.[1] ?? 0))} />
                </li>
              ))}
              {f.count > f.lines.length && (
                <li className="pl-8 text-2xs text-muted-foreground">{plural(f.count - f.lines.length, 'more line')} in this file</li>
              )}
            </ul>
          </Disclosure>
        ))}
      </div>
    </>
  );
}

/** One matching line: its number, then the line with the matches marked. */
function MatchRow({ line: l, onOpen }: { line: SearchLine; onOpen: () => void }) {
  const parts: ReactNode[] = [];
  // The indent is left out: the match is what matters in a narrow column.
  let at = l.offset > 0 ? 0 : l.text.length - l.text.trimStart().length;
  for (const [from, to] of l.ranges) {
    if (to <= at) continue;
    if (from > at) parts.push(l.text.slice(at, from));
    parts.push(<mark key={from} className="rounded-sm bg-waiting-soft text-foreground">{l.text.slice(Math.max(from, at), to)}</mark>);
    at = to;
  }
  parts.push(l.text.slice(at));
  return (
    <button type="button" onClick={onOpen} title={`Line ${l.line}`}
      className="flex w-full min-w-0 items-baseline gap-2 rounded-sm py-0.5 pr-1 pl-2 text-left font-mono text-xs hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring">
      <span className="w-8 shrink-0 text-right text-2xs text-muted-foreground">{l.line}</span>
      <span className="min-w-0 truncate whitespace-pre">
        {l.offset > 0 && '…'}{parts}{l.cutEnd && '…'}
      </span>
    </button>
  );
}
