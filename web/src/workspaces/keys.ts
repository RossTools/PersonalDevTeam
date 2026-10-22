import { useEffect, useRef } from 'react';

/** The prefix: Ctrl+] (unbound in zsh/readline, Claude Code and Codex; browsers leave it alone). */
export function isPrefix(e: KeyboardEvent): boolean {
  return e.ctrlKey && !e.altKey && !e.metaKey && (e.key === ']' || e.code === 'BracketRight');
}

export type KeyAction =
  | { type: 'newTab' }
  | { type: 'tab'; index: number }
  | { type: 'next' }
  | { type: 'prev' }
  | { type: 'switcher' }
  | { type: 'close' }
  | { type: 'rename' }
  | { type: 'needsYou' }
  | { type: 'palette' }
  | { type: 'backlog' }
  | { type: 'board' }
  | { type: 'inbox' }
  | { type: 'pm' }
  | { type: 'help' };

/** The action for the key pressed after the prefix, or null for an unbound key. */
export function actionFor(key: string): KeyAction | null {
  if (/^[1-9]$/.test(key)) return { type: 'tab', index: Number(key) - 1 };
  switch (key) {
    case 'c': return { type: 'newTab' };
    case 'n': return { type: 'next' };
    case 'p': return { type: 'prev' };
    case 'w': return { type: 'switcher' };
    case 'x': return { type: 'close' };
    case 'r': return { type: 'rename' };
    case 'a': return { type: 'needsYou' };
    case 'k': return { type: 'palette' };
    case 'l': return { type: 'backlog' };
    case 'b': return { type: 'board' };
    case 'i': return { type: 'inbox' };
    case 'm': return { type: 'pm' };
    case '?': return { type: 'help' };
    default: return null;
  }
}

export const KEY_HELP: [string, string][] = [
  ['c', 'New tab'],
  ['1–9', 'Go to tab N'],
  ['n / p', 'Next / previous tab'],
  ['w', 'Switch workspace'],
  ['x', 'Close the tab (the session keeps running)'],
  ['r', 'Rename the tab'],
  ['a', 'Next session that needs you'],
  ['k', 'Command palette (also ⌘K on a Mac)'],
  ['l', 'Tickets (backlog)'],
  ['b', 'Tickets (board)'],
  ['i', 'Inbox (then j / k, Enter, o, e)'],
  ['m', 'Project manager'],
  ['?', 'Show these keys'],
  ['Ctrl+] twice', 'Send Ctrl+] to the terminal'],
];

const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta']);

/**
 * Listens for the prefix, then one key, on the window in the capture phase, so the terminal never sees either.
 * Pressing the prefix twice lets the second one through to whatever has focus (the terminal sends it as 0x1d).
 */
export function usePrefixKeys(onAction: (a: KeyAction) => void) {
  const armed = useRef(false);
  const handler = useRef(onAction);
  handler.current = onAction;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (armed.current) {
        if (MODIFIER_KEYS.has(e.key)) return;
        armed.current = false;
        if (isPrefix(e)) return; // second prefix: let the terminal have it
        e.preventDefault();
        e.stopPropagation();
        const action = actionFor(e.key);
        if (action) handler.current(action);
        return;
      }
      if (isPrefix(e)) {
        e.preventDefault();
        e.stopPropagation();
        armed.current = true;
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);
}
