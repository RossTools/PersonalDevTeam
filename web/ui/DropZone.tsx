import { useRef, useState, type DragEvent, type ReactNode } from 'react';
import { Upload } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button } from './Button';

/** A file being dragged: its name when the browser says (most say only on drop), and its type ("text/csv", or ""). */
export interface FileDrag {
  name: string | null;
  type: string;
}

export interface FileDrop {
  /** The file dragged over the area, while one is. */
  drag: FileDrag | null;
  /** Spread on the area that takes the drop. */
  dropProps: {
    onDragEnter: (e: DragEvent) => void;
    onDragOver: (e: DragEvent) => void;
    onDragLeave: (e: DragEvent) => void;
    onDrop: (e: DragEvent) => void;
  };
}

const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');

/**
 * Files dropped on an area (a whole dialog): pass the result to a `DropZone` as `drop`, so the zone shows the drag and
 * the area's drop lands in it. Drags of anything but files are left alone.
 */
export function useFileDrop(onFile: (file: File) => void, disabled = false): FileDrop {
  const [drag, setDrag] = useState<FileDrag | null>(null);
  // dragenter and dragleave fire for every child crossed: count them, so the drag ends only on leaving the area.
  const depth = useRef(0);
  const end = () => {
    depth.current = 0;
    setDrag(null);
  };
  return {
    drag,
    dropProps: {
      onDragEnter: (e) => {
        if (disabled || !hasFiles(e)) return;
        e.preventDefault();
        depth.current += 1;
        const item = e.dataTransfer.items?.[0];
        setDrag({ name: e.dataTransfer.files?.[0]?.name ?? null, type: item?.type ?? '' });
      },
      onDragOver: (e) => {
        if (disabled || !hasFiles(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = 'copy';
      },
      onDragLeave: (e) => {
        if (disabled || !hasFiles(e)) return;
        depth.current -= 1;
        if (depth.current <= 0) end();
      },
      onDrop: (e) => {
        if (disabled || !hasFiles(e)) return;
        e.preventDefault();
        end();
        const f = e.dataTransfer.files?.[0];
        if (f) onFile(f);
      },
    },
  };
}

export interface DropZoneProps {
  /** Called with the file dropped or chosen. */
  onFile: (file: File) => void;
  /** The file chooser's `accept` (".md,.xlsx"). */
  accept?: string;
  /** What to drop, in the zone ("Drop a .md (becomes a doc)…"). Hidden on phones, where the zone is its button. */
  children: ReactNode;
  /** While a file is dragged over: what dropping it does ("Drop to import Q4 plan.xlsx as a sheet"). */
  describe?: (drag: FileDrag) => ReactNode;
  /** The button that opens the file chooser. */
  chooseLabel?: string;
  /** Taking a file: the button shows it, and nothing more is taken. */
  pending?: boolean;
  disabled?: boolean;
  /** A larger area's drag (`useFileDrop` on a dialog): the zone shows it, and takes no drops of its own. */
  drop?: FileDrop;
  className?: string;
}

/**
 * A dashed area to drop a file on, with a button that chooses one. Dragging a file over it (or over the area whose
 * `drop` it is given) highlights it and says what dropping does. On phones it is just the button.
 */
export function DropZone({
  onFile,
  accept,
  children,
  describe,
  chooseLabel = 'Choose file…',
  pending = false,
  disabled = false,
  drop,
  className,
}: DropZoneProps) {
  const off = disabled || pending;
  const own = useFileDrop(onFile, off || drop !== undefined);
  const drag = (drop ?? own).drag;
  const input = useRef<HTMLInputElement>(null);
  return (
    <div
      data-slot="drop-zone"
      data-dragging={drag ? '' : undefined}
      {...(drop ? {} : own.dropProps)}
      className={cn(
        'flex items-center gap-3 rounded-lg border border-dashed border-input px-3 py-2.5 text-sm text-muted-foreground transition-colors duration-(--pdt-duration) motion-reduce:transition-none',
        'max-sm:border-0 max-sm:p-0',
        drag && 'border-ring bg-accent text-foreground',
        className,
      )}
    >
      <Upload aria-hidden className="size-4 shrink-0 max-sm:hidden" />
      <span aria-live="polite" className="min-w-0 flex-1 max-sm:hidden">
        {drag ? (describe?.(drag) ?? 'Drop to import it') : children}
      </span>
      {!drag && (
        <Button variant="secondary" size="sm" pending={pending} disabled={disabled} onClick={() => input.current?.click()}>
          {chooseLabel}
        </Button>
      )}
      <input
        ref={input}
        type="file"
        accept={accept}
        aria-label="File to import"
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = '';
          if (f) onFile(f);
        }}
      />
    </div>
  );
}
