import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Eye, ImagePlus, Pencil } from 'lucide-react';
import { Alert, Button, SegmentedControl } from '@ui';
import { api, ApiError } from '../api/client';
import type { DocumentRead } from '../api/gen/DocumentRead';
import { MarkdownEditor } from '../common/LazyMarkdownEditor';
import { Markdown, MarkdownImageContext } from '../common/Markdown';
import { actionError, runAction } from '../common/useAction';
import { documentKeys } from './queries';
import { useDocumentEvents } from './useDocumentEvents';

type Mode = 'read' | 'write';

const MODES = [
  { value: 'read' as const, label: 'Rendered', icon: <Eye aria-hidden className="size-3" /> },
  { value: 'write' as const, label: 'Write', icon: <Pencil aria-hidden className="size-3" /> },
];

/** How long typing pauses before the text is saved. */
const SAVE_AFTER = 800;

export interface DocEditorProps {
  id: string;
  markdown: string;
  version: string | null;
  /** Only PDT's local docs are edited here (saved as typed, with live updates and images). */
  editable: boolean;
}

/**
 * A doc (PDT-315): Rendered or Write. Write saves as you type, on top of the version it opened; when someone else (an
 * agent) saved it in between, nothing is overwritten and a notice offers to keep yours or take theirs. Changes saved
 * elsewhere show at once while you have nothing unsaved. Images (Add image) are saved beside the doc.
 */
export function DocEditor({ id, markdown, version, editable }: DocEditorProps) {
  const qc = useQueryClient();
  const [mode, setMode] = useState<Mode>('read');
  const [text, setText] = useState(markdown);
  // The version the text on screen is based on, and whether it has changes not saved yet.
  const base = useRef(version);
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const textRef = useRef(text);
  textRef.current = text;
  const imageInput = useRef<HTMLInputElement>(null);

  // A newer read from pdtd replaces the text, unless the user has changes of their own.
  useEffect(() => {
    if (dirty || version === base.current) return;
    base.current = version;
    setText(markdown);
  }, [markdown, version, dirty]);

  const reread = useCallback(() => qc.invalidateQueries({ queryKey: documentKeys.content(id) }), [qc, id]);

  const save = useCallback(async (overwrite = false) => {
    setSaving(true);
    try {
      const r = await api.editDocument(id, [{ op: 'replace_text', markdown: textRef.current }], overwrite ? null : base.current);
      base.current = r.version;
      setDirty(false);
      setConflict(false);
      setSaveError(null);
      qc.setQueryData<DocumentRead>(documentKeys.content(id), (d) =>
        d && d.content.kind === 'doc' ? { ...d, content: { ...d.content, markdown: textRef.current, version: r.version } } : d);
    } catch (e) {
      if (e instanceof ApiError && e.code === 'conflict') setConflict(true);
      else setSaveError(actionError(e));
    } finally {
      setSaving(false);
    }
  }, [id, qc]);

  // Save a pause after typing (not while a conflict waits for the user).
  useEffect(() => {
    if (!dirty || conflict || saving) return;
    const t = setTimeout(() => void save(), SAVE_AFTER);
    return () => clearTimeout(t);
  }, [dirty, conflict, saving, text, save]);

  useDocumentEvents(id, editable, (e) => {
    if (e.type !== 'changed' || e.version === base.current) return;
    if (dirty) setConflict(true);
    else void reread();
  }, () => void reread());

  const takeTheirs = async () => {
    setDirty(false);
    setConflict(false);
    base.current = null;
    await reread();
  };

  const addImage = (file: File) => void runAction(async () => {
    const { path } = await api.uploadDocumentImage(id, file);
    const alt = file.name.replace(/\.[^.]+$/, '');
    const next = `${textRef.current.replace(/\n*$/, '')}\n\n![${alt}](${path})\n`;
    setText(next);
    setDirty(true);
    setMode('write');
  });

  const resolveImage = useCallback((src: string) => api.documentAssetUrl(id, src), [id]);

  return (
    <div className="flex flex-col gap-2">
      {editable && (
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl aria-label="View" size="sm" options={MODES} value={mode} onChange={(m) => setMode(m)} />
          {mode === 'write' && (
            <>
              <Button variant="ghost" size="sm" compact onClick={() => imageInput.current?.click()}><ImagePlus aria-hidden /> Add image</Button>
              <input
                ref={imageInput}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml"
                className="hidden"
                aria-label="Image to add"
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  e.target.value = '';
                  if (f) addImage(f);
                }}
              />
            </>
          )}
          <span className="ml-auto text-meta text-muted-foreground" aria-live="polite">
            {saving ? 'Saving…' : dirty ? 'Unsaved changes' : base.current ? 'Saved' : ''}
          </span>
        </div>
      )}
      {conflict && (
        <Alert tone="warning" title="This document changed while you were writing">
          <p>Someone else, or an agent, saved it. Keep your version (theirs is replaced), or take theirs (your changes go).</p>
          <div className="mt-2 flex gap-2">
            <Button size="sm" onClick={() => void save(true)}>Keep mine</Button>
            <Button size="sm" variant="secondary" onClick={() => void takeTheirs()}>Take theirs</Button>
          </div>
        </Alert>
      )}
      {saveError && <Alert tone="error">Not saved: {saveError}</Alert>}
      {mode === 'write' && editable ? (
        <MarkdownEditor
          label="Document text"
          value={text}
          onChange={(t) => {
            setText(t);
            setDirty(true);
          }}
          onBlur={() => {
            if (dirty && !conflict) void save();
          }}
          autoFocus="end"
          className="min-h-64 rounded-md border border-border bg-card p-3"
        />
      ) : text.trim() ? (
        <MarkdownImageContext.Provider value={editable ? resolveImage : null}>
          <Markdown variant="document">{text}</Markdown>
        </MarkdownImageContext.Provider>
      ) : (
        <p className="text-xs text-muted-foreground">This document is empty.{editable && ' Switch to Write to add to it.'}</p>
      )}
    </div>
  );
}
