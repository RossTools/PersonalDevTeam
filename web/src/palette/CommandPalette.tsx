import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import {
  ArrowRightLeft, BellRing, Bot, Check, Code, Keyboard, Layers, PanelTop, Pencil, Play, Plus, Server, Settings, Sparkles,
  SquareTerminal, SunMoon, X,
} from 'lucide-react';
import { Button, CommandList, Dialog, Field, Input, type CommandItem } from '@ui';
import { AgentDot } from '../common/AgentState';
import type { PaletteIcon, PaletteItem, PaletteStep } from './paletteItems';

const ICONS: Record<PaletteIcon, ReactNode> = {
  workspace: <Layers aria-hidden />,
  tab: <PanelTop aria-hidden />,
  agent: <Bot aria-hidden />,
  settings: <Settings aria-hidden />,
  host: <Server aria-hidden />,
  claude: <Sparkles aria-hidden />,
  codex: <Code aria-hidden />,
  shell: <SquareTerminal aria-hidden />,
  run: <Play aria-hidden />,
  rename: <Pencil aria-hidden />,
  close: <X aria-hidden />,
  move: <ArrowRightLeft aria-hidden />,
  needsYou: <BellRing aria-hidden />,
  theme: <SunMoon aria-hidden />,
  keys: <Keyboard aria-hidden />,
  check: <Check aria-hidden />,
  add: <Plus aria-hidden />,
};

/** CommandList has no secondary-text slot: the detail (a tab's workspace) follows the label and is filtered with it. */
function toCommand(it: PaletteItem): CommandItem {
  return {
    id: it.id,
    label: it.detail ? `${it.label} — ${it.detail}` : it.label,
    group: it.group,
    ...(it.shortcut ? { shortcut: it.shortcut } : {}),
    ...(it.keywords ? { keywords: it.keywords } : {}),
    icon: it.state ? <AgentDot state={it.state} className="w-4 justify-center" /> : it.icon ? ICONS[it.icon] : undefined,
  };
}

function NameStep({ step, onDone }: { step: Extract<PaletteStep, { kind: 'input' }>; onDone: (value: string) => void }) {
  const [value, setValue] = useState('');
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const name = value.trim();
  return (
    <form
      className="flex flex-col gap-3 p-1"
      onSubmit={(e) => {
        e.preventDefault();
        if (name) onDone(name);
      }}
    >
      <Field label={step.label}>
        <Input ref={input} value={value} placeholder={step.placeholder} onChange={(e) => setValue(e.target.value)} />
      </Field>
      <div className="flex justify-end">
        <Button type="submit" variant="primary" disabled={!name}>{step.submitLabel}</Button>
      </div>
    </form>
  );
}

/**
 * The command palette (Ctrl+] k, ⌘K on macOS): a filterable list of everything to go to and do. Enter (or a click)
 * runs the highlighted item and closes; an item with a second step (Move tab to workspace…) swaps the list. Escape
 * clears the filter, then goes back a step, then closes; focus returns to what had it.
 */
export function CommandPalette({ open, onOpenChange, items }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: PaletteItem[];
}) {
  const [steps, setSteps] = useState<PaletteStep[]>([]);
  useEffect(() => {
    if (open) setSteps([]);
  }, [open]);
  const step = steps[steps.length - 1];
  const list = step?.kind === 'list' ? step.items : items;

  const finish = (run: () => void) => {
    onOpenChange(false);
    run();
  };
  const choose = (id: string) => {
    const it = list.find((i) => i.id === id);
    if (!it) return;
    if (it.next) setSteps((s) => [...s, it.next!]);
    else if (it.run) finish(it.run);
  };
  // Escape inside a step goes back instead of closing. A filter with text keeps its own Escape (it clears the text).
  const onKeyDownCapture = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Escape' || steps.length === 0) return;
    const target = e.target as HTMLInputElement;
    if (target.getAttribute('role') === 'searchbox' && target.value) return;
    e.preventDefault();
    e.stopPropagation();
    setSteps((s) => s.slice(0, -1));
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={step?.title ?? 'Command palette'}
      size="lg"
      className="top-4 translate-y-0 sm:top-[12vh]"
    >
      <div onKeyDownCapture={onKeyDownCapture}>
        {step?.kind === 'input' ? (
          <NameStep step={step} onDone={(v) => finish(() => step.run(v))} />
        ) : (
          <CommandList
            key={steps.length}
            label={step?.title ?? 'Commands'}
            placeholder={step?.placeholder ?? 'Go to or do anything…'}
            items={list.map(toCommand)}
            onAction={choose}
            className="max-h-[60dvh]"
          />
        )}
      </div>
    </Dialog>
  );
}
