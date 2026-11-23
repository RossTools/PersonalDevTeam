// Preview | Edit (PDT-679): the one control for viewing or editing Markdown, the same order and icons on notes,
// documents, files and personas. Files add Source (the file's text, read-only) for Markdown.
import { Code, Eye, Pencil } from 'lucide-react';
import { SegmentedControl, type SegmentedOption } from '@ui';

export type PreviewEditMode = 'preview' | 'edit' | 'source';

const PREVIEW: SegmentedOption<'preview'> = { value: 'preview', label: 'Preview', icon: <Eye aria-hidden className="size-3" /> };
const EDIT: SegmentedOption<'edit'> = { value: 'edit', label: 'Edit', icon: <Pencil aria-hidden className="size-3" /> };
const SOURCE: SegmentedOption<'source'> = { value: 'source', label: 'Source', icon: <Code aria-hidden className="size-3" /> };

export interface PreviewEditProps<V extends PreviewEditMode> {
  value: V;
  onChange: (value: V) => void;
  /** Leave Edit out (a read-only file or document). */
  noEdit?: boolean;
  /** Add Source after Edit (a Markdown file). */
  source?: boolean;
  'aria-label'?: string;
  className?: string;
}

/** Preview | Edit (| Source). */
export function PreviewEdit<V extends PreviewEditMode>({ value, onChange, noEdit, source, 'aria-label': label = 'View', className }: PreviewEditProps<V>) {
  const options = [PREVIEW, ...(noEdit ? [] : [EDIT]), ...(source ? [SOURCE] : [])] as SegmentedOption<V>[];
  return <SegmentedControl aria-label={label} size="sm" options={options} value={value} onChange={onChange} className={className} />;
}
