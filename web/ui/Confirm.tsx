import { createContext, useCallback, useContext, useId, useRef, useState, type ReactNode } from 'react';
import { CheckIcon } from 'lucide-react';
import { Checkbox as AriaCheckbox } from 'react-aria-components';
import {
  AlertDialog,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from './primitives/alert-dialog';
import { Button } from './Button';
import { Input } from './Input';

export interface ConfirmOptions {
  title: ReactNode;
  /** What is lost (spec §6.2). */
  body?: ReactNode;
  /** The confirming button's label: names the action ("Remove worktree", never "OK"). */
  action: string;
  /** Uses the danger variant (irreversible actions). */
  danger?: boolean;
  /** Defaults to "Cancel". */
  cancel?: string;
  /** An extra choice, e.g. "Also delete the branch". */
  checkbox?: { label: ReactNode; defaultChecked?: boolean };
  /** A name the user types before the action is enabled (removing a project): for what cannot be undone. */
  typeToConfirm?: string;
}

export interface ConfirmResult {
  ok: boolean;
  checked: boolean;
}

type Confirm = (o: ConfirmOptions) => Promise<ConfirmResult>;

const ConfirmContext = createContext<Confirm | null>(null);

/** `const confirm = useConfirm(); const { ok, checked } = await confirm({ title, body, action, danger })`. */
export function useConfirm(): Confirm {
  const c = useContext(ConfirmContext);
  if (!c) throw new Error('useConfirm needs a <ConfirmProvider> above it');
  return c;
}

interface Pending {
  options: ConfirmOptions;
  resolve: (r: ConfirmResult) => void;
}

/** Render once near the app root; it draws the confirm dialog that `useConfirm` opens. */
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const [checked, setChecked] = useState(false);
  const [typed, setTyped] = useState('');
  const current = useRef<Pending | null>(null);
  // Kept after closing so the dialog keeps its text while it animates out.
  const last = useRef<ConfirmOptions | null>(null);
  const descId = useId();

  const finish = useCallback((ok: boolean, isChecked: boolean) => {
    const p = current.current;
    current.current = null;
    setPending(null);
    p?.resolve({ ok, checked: isChecked });
  }, []);

  const confirm = useCallback<Confirm>(
    (options) =>
      new Promise<ConfirmResult>((resolve) => {
        // A second confirm while one is open cancels the first.
        current.current?.resolve({ ok: false, checked: false });
        const p = { options, resolve };
        last.current = options;
        current.current = p;
        setChecked(options.checkbox?.defaultChecked ?? false);
        setTyped('');
        setPending(p);
      }),
    [],
  );

  const o = pending?.options ?? last.current;
  const typedOk = !o?.typeToConfirm || typed.trim() === o.typeToConfirm;
  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      <AlertDialog
        isOpen={pending !== null}
        onOpenChange={(open) => {
          if (!open) finish(false, checked);
        }}
        isDismissable={false}
        dialogProps={o?.body ? { 'aria-describedby': descId } : undefined}
        className="gap-3 data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-md"
      >
        {o && (
          <>
            <AlertDialogHeader className="place-items-start text-left">
              <AlertDialogTitle className="text-sm font-semibold">{o.title}</AlertDialogTitle>
              {o.body && (
                <AlertDialogDescription id={descId} className="text-xs text-muted-foreground">
                  {o.body}
                </AlertDialogDescription>
              )}
            </AlertDialogHeader>
            {o.checkbox && (
              <AriaCheckbox
                isSelected={checked}
                onChange={setChecked}
                className="group flex w-fit cursor-default items-center gap-2 text-sm outline-none pointer-coarse:min-h-control-touch"
              >
                <span
                  aria-hidden
                  className="flex size-4 shrink-0 items-center justify-center rounded-sm border border-input text-primary-foreground group-data-focus-visible:ring-2 group-data-focus-visible:ring-ring group-data-selected:border-primary group-data-selected:bg-primary"
                >
                  <CheckIcon className="hidden size-3 group-data-selected:block" />
                </span>
                {o.checkbox.label}
              </AriaCheckbox>
            )}
            {o.typeToConfirm && (
              <label className="flex flex-col gap-1.5 text-xs text-muted-foreground">
                <span>Type <span className="font-mono text-foreground">{o.typeToConfirm}</span> to confirm.</span>
                <Input aria-label={`Type ${o.typeToConfirm} to confirm`} value={typed} onChange={(e) => setTyped(e.target.value)} />
              </label>
            )}
            <AlertDialogFooter>
              <Button variant="secondary" autoFocus={o.danger && !o.typeToConfirm} onClick={() => finish(false, checked)}>
                {o.cancel ?? 'Cancel'}
              </Button>
              <Button
                variant={o.danger ? 'danger' : 'primary'}
                autoFocus={!o.danger}
                disabled={!typedOk}
                onClick={() => finish(true, checked)}
              >
                {o.action}
              </Button>
            </AlertDialogFooter>
          </>
        )}
      </AlertDialog>
    </ConfirmContext.Provider>
  );
}
