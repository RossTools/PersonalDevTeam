import type { ReactNode } from 'react';
import { cn } from './lib/utils';
import { Checkbox } from './Checkbox';

export interface CheckTableColumn<T> {
  id: string;
  header: ReactNode;
  cell: (row: T, n: number) => ReactNode;
  /** Hidden on phones (below `sm`): another column shows what matters of it there. */
  wide?: boolean;
}

/**
 * Rows to keep or drop, as a table with a checkbox first in each row: an unticked row is muted and struck through.
 * The caller keeps which rows are ticked.
 */
export function CheckTable<T>({ rows, rowKey, columns, checked, onCheck, checkLabel, checkHeader = 'Keep', disabled, className, 'aria-label': ariaLabel }: {
  rows: T[];
  rowKey: (row: T) => string;
  columns: CheckTableColumn<T>[];
  /** Whether row `n` is ticked. */
  checked: (n: number) => boolean;
  onCheck: (n: number, on: boolean) => void;
  /** Names a row's checkbox ("Keep PDT-12"). */
  checkLabel: (row: T) => string;
  /** Names the checkbox column (read by screen readers only). */
  checkHeader?: string;
  disabled?: boolean;
  className?: string;
  'aria-label': string;
}) {
  const wide = (c: CheckTableColumn<T>) => c.wide && 'max-sm:hidden';
  return (
    <div className={cn('overflow-x-auto rounded-lg border border-border bg-card', className)}>
      <table className="w-full text-sm" aria-label={ariaLabel}>
        <thead>
          <tr className="text-left text-xs text-muted-foreground">
            <th className="w-8 px-3 py-2 font-medium"><span className="sr-only">{checkHeader}</span></th>
            {columns.map((c) => <th key={c.id} className={cn('px-3 py-2 font-medium', wide(c))}>{c.header}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((r, n) => (
            <tr key={rowKey(r)} className={cn('border-t border-border', !checked(n) && 'text-muted-foreground line-through')}>
              <td className="px-3 py-2 align-top">
                <Checkbox aria-label={checkLabel(r)} className="justify-center pointer-coarse:min-w-control-touch" checked={checked(n)} disabled={disabled} onChange={(on) => onCheck(n, on)} />
              </td>
              {columns.map((c) => <td key={c.id} className={cn('px-3 py-2 align-top', wide(c))}>{c.cell(r, n)}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
