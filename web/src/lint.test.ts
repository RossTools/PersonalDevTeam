// @vitest-environment node
// Guard rails for the design system: colours come from tokens.css, sizes and
// layers from the theme, confirmations from useConfirm, and features build on '@ui' rather than the libraries under it.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const web = dirname(dirname(fileURLToPath(new URL('./lint.test.ts', import.meta.url))));

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'gen' || name === 'node_modules' ? [] : files(path);
    return /\.(tsx?|css)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

const sources = [...files(join(web, 'src')), ...files(join(web, 'ui'))]
  // Forward slashes on every OS, so the path checks below work on Windows too.
  .map((path) => ({ path: relative(web, path).split(sep).join('/'), text: readFileSync(path, 'utf8') }))
  .filter(({ path }) => path !== 'ui/tokens.css' && path !== 'ui/shadcn.css' && path !== 'src/lint.test.ts');

function offenders(pattern: RegExp, only: (path: string) => boolean = () => true) {
  return sources
    .filter(({ path }) => only(path))
    .flatMap(({ path, text }) =>
      text.split('\n').flatMap((line, i) => (pattern.test(line) ? [`${path}:${i + 1}: ${line.trim()}`] : [])),
    );
}

/**
 * Files still waiting to move onto the rule: a rule's offenders outside them must be none, and each file listed must
 * still offend, so the list shrinks as callers move.
 */
function expectOnly(found: string[], allowed: string[]) {
  const file = (line: string) => line.slice(0, line.indexOf(':'));
  expect(found.filter((line) => !allowed.includes(file(line)))).toEqual([]);
  expect(allowed.filter((path) => !found.some((line) => file(line) === path))).toEqual([]);
}

const outsideUi = (p: string) => !p.startsWith('ui/');

describe('design system rules', () => {
  it('defines colours only in tokens.css', () => {
    // Hex and rgb()/hsl() literals, and black/white utilities. CodeViewer and the terminal theme read tokens through
    // var(--pdt-*), which is fine.
    expect(
      offenders(
        /#[0-9a-fA-F]{3,8}\b(?![\w-])|\b(rgba?|hsla?)\(|(?<![\w-])(bg|text|border|divide|ring|ring-offset|outline|fill|stroke|shadow|from|via|to|caret|accent|decoration|placeholder)-(black|white)\b/,
      ),
    ).toEqual([]);
    // Tailwind's own palette (`bg-red-500`, `text-zinc-400`) bypasses the tokens and their dark-mode values.
    const palette = 'red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|slate|gray|zinc|neutral|stone';
    expect(offenders(new RegExp(`(?<![\\w-])[a-z-]+-(${palette})-\\d{2,3}\\b`))).toEqual([]);
  });

  it('colours accent text with text-link: text-primary is the fill colour, too light for text in dark mode', () => {
    expect(offenders(/(?<![\w-])text-primary(?![\w-])/)).toEqual([]);
  });

  it('uses theme sizes and layers, not arbitrary ones', () => {
    // Any arbitrary type size (`text-[13px]`, `text-[0.8rem]/5`) and layer.
    expect(offenders(/\btext-\[[\d.]|\bz-\[/)).toEqual([]);
    // App layers come from --pdt-z-*; a small z-10 inside a primitive only stacks that component's own parts.
    expect(offenders(/(?<![\w-])z-\d+\b/, (p) => !p.startsWith('ui/primitives/'))).toEqual([]);
  });

  it('keeps type to the scale: text-lg and up are headings for @ui', () => {
    // No heading size token yet: these pages still set their own.
    expectOnly(offenders(/(?<![\w-])text-(lg|\d?xl)(?![\w-])/, outsideUi), [
      'src/documents/DocumentPage.tsx',
      'src/plugins/parts.tsx',
      'src/setup/SetupPage.tsx',
      'src/todos/TodoView.tsx',
    ]);
  });

  it('shadows with shadow-popover, shadow-dialog or shadow-card, not Tailwind\'s scale', () => {
    expect(offenders(/(?<![\w-])shadow-(2xs|xs|sm|md|lg|xl|2xl|inner)(?![\w-])/)).toEqual([]);
  });

  it('sizes touch controls with control-touch and control-lg, not h-11 or h-12', () => {
    // h-12 that isn't a control (a thumbnail, a bar, a textarea's starting height) waits here for a better name.
    expectOnly(offenders(/(?<![\w-])(min-|max-)?(h|size)-1[12](?![\w-])/), [
      'src/host/BugReportDialog.tsx',
      'src/plugins/parts.tsx',
      'src/sessions/EndedBar.tsx',
      'src/tickets/TicketsView.tsx',
      'ui/ImageThumb.tsx',
    ]);
  });

  it('fills hover with bg-accent, the soft hover fill', () => {
    expectOnly(offenders(/(?<![\w-])hover:bg-(muted|surface-2)(?![\w-])/, outsideUi), [
      'src/documents/BesidePanel.tsx',
      'src/notes/NotePage.tsx',
      'src/plugins/AllConnectionsPage.tsx',
      'src/plugins/PluginsPage.tsx',
      'src/shell/RightPanel.tsx',
      'src/tickets/Attempt.tsx',
      'src/tickets/TicketConversation.tsx',
      'src/todos/DuePicker.tsx',
      'src/todos/TodoRow.tsx',
      'src/workflows/NewWorkflowDialog.tsx',
      'src/workspaces/WorkspaceBar.tsx',
    ]);
  });

  it('labels a group with SectionLabel, not a hand-made uppercase tracked label', () => {
    const tracked = /(?<![\w-])uppercase(?![\w-]).*(?<![\w-])tracking-|(?<![\w-])tracking-.*(?<![\w-])uppercase(?![\w-])/;
    expect(offenders(tracked, outsideUi)).toEqual([]);
  });

  it('confirms with useConfirm, never window.confirm', () => {
    expect(offenders(/\bwindow\.confirm\(|\bconfirm\(\s*['"`]/)).toEqual([]);
  });

  it("merges classes with web/ui/lib/utils' cn, which knows our theme names", () => {
    expect(offenders(/from ['"]cn['"]/)).toEqual([]);
  });

  it('turns movement off under motion-reduce', () => {
    // Animations, transforms and layout transitions: the same class string also says `motion-reduce:`. Colour and
    // opacity transitions don't move anything; index.css's reduced-motion rule shortens them too.
    const moves = /(?<![\w-])(animate-(in|out|spin|pulse|pulse-slow|bounce|ping)|transition-transform|transition-\[[^\]]+\]|transition)(?=[\s'"`])/;
    expect(
      offenders(moves, (p) => p.endsWith('.tsx')).filter((line) => !line.includes('motion-reduce:')),
    ).toEqual([]);
  });

  it('builds overlays from @ui, never a hand-rolled role="dialog", menu, listbox or tooltip', () => {
    // @ui's Dialog, Sheet, Popover, Menu, Select and Tooltip move focus in, trap or restore it and close on Escape. A
    // CSS selector naming the role (`[role="dialog"]`) is fine.
    expect(offenders(/(?<![[\w-])role=\{?['"`](alertdialog|dialog|menu|listbox|tooltip)['"`]/, outsideUi)).toEqual([]);
  });

  it('builds tabs from Tabs and folds with Disclosure, never <details>', () => {
    // Editor-style tabs that are dragged, renamed or closed are the one hand-built tab list (web/ui/README.md).
    expectOnly(offenders(/(?<![[\w-])role=\{?['"`](tab|tablist|tabpanel)['"`]/, outsideUi), [
      'src/documents/BesidePanel.tsx',
      'src/workspaces/WorkspaceBar.tsx',
    ]);
    expect(offenders(/<details[\s>]/, outsideUi)).toEqual([]);
  });

  it('keeps React Aria, Base UI, React Flow and the primitives behind @ui', () => {
    expect(
      offenders(/from ['"](react-aria-components|react-aria|react-stately|@base-ui\/react[^'"]*|@xyflow\/react[^'"]*|@ui\/primitives[^'"]*)['"]/, (p) => !p.startsWith('ui/')),
    ).toEqual([]);
  });
});
