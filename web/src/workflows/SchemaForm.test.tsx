import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { editorText, setEditorText } from '../test-utils';
import { SchemaForm, classify } from './SchemaForm';

// PDT-250: a schema's `x-format` picks the editor for long text.
const SCHEMA = {
  type: 'object',
  properties: {
    prompt: { type: 'string', 'x-format': 'markdown' },
    script: { type: ['string', 'null'], 'x-format': 'code', 'x-language': 'sh' },
    input: { default: null, 'x-format': 'code', 'x-language': 'json' },
    body: { type: 'string', 'x-format': 'multiline' },
    title: { type: 'string' },
  },
};

const field = (key: string) => classify((SCHEMA.properties as Record<string, Record<string, unknown>>)[key], SCHEMA).field;

/** The form, holding its value; `seen` gets each change. */
function Form({ start, seen, readOnly }: { start: Record<string, unknown>; seen: (v: Record<string, unknown>) => void; readOnly?: boolean }) {
  const [value, setValue] = useState(start);
  return <SchemaForm schema={SCHEMA} value={value} readOnly={readOnly} onChange={(v) => { setValue(v); seen(v); }} />;
}

describe('format hints', () => {
  it('pick the editor', () => {
    expect(field('prompt')).toEqual({ kind: 'markdown' });
    expect(field('script')).toEqual({ kind: 'code', language: 'sh' });
    expect(field('input')).toEqual({ kind: 'json' });
    expect(field('body')).toEqual({ kind: 'string', multiline: true });
    // Without a hint, text is one line (no more guessing from the field's name).
    expect(field('title')).toEqual({ kind: 'string', multiline: false });
    expect(classify({ type: 'string' }, {}).field).toEqual({ kind: 'string', multiline: false });
  });

  it('render a Markdown editor, a code editor, JSON as code and a growing text area', async () => {
    const seen = vi.fn();
    render(<Form start={{ prompt: '# Task', script: 'echo hi', input: { n: 1 }, body: 'a', title: 't' }} seen={seen} />);
    const prompt = await screen.findByRole('textbox', { name: 'Prompt' });
    expect(prompt.closest('[data-testid="markdown-editor"]')).not.toBeNull();
    expect(editorText(prompt)).toBe('# Task');
    const script = await screen.findByRole('textbox', { name: 'Script' });
    expect(script.closest('[data-testid="code-editor"]')).not.toBeNull();
    expect(editorText(script)).toBe('echo hi');
    expect(editorText(screen.getByRole('textbox', { name: 'Input' }))).toBe('{\n  "n": 1\n}');
    expect(screen.getByRole('textbox', { name: 'Body' }).tagName).toBe('TEXTAREA');

    setEditorText(script, 'echo one\necho two');
    expect(seen).toHaveBeenLastCalledWith(expect.objectContaining({ script: 'echo one\necho two' }));
    setEditorText(screen.getByRole('textbox', { name: 'Input' }), '{"n": 2}');
    expect(seen).toHaveBeenLastCalledWith(expect.objectContaining({ input: { n: 2 } }));
    // Text that isn't JSON yet is kept, with why.
    setEditorText(screen.getByRole('textbox', { name: 'Input' }), '{"n": ');
    expect(editorText(screen.getByRole('textbox', { name: 'Input' }))).toBe('{"n": ');
    expect(screen.getByText(/JSON/)).toBeInTheDocument();
    // Cleared optional text is left out.
    setEditorText(script, '');
    expect(seen.mock.lastCall![0]).not.toHaveProperty('script');
  });
});

describe('expand', () => {
  it('opens the field in a large dialog; Done keeps the edit', async () => {
    const seen = vi.fn();
    render(<Form start={{ prompt: 'Short' }} seen={seen} />);
    await userEvent.click(screen.getByRole('button', { name: 'Expand Prompt' }));
    const dialog = screen.getByRole('dialog', { name: 'Prompt' });
    const big = await within(dialog).findByRole('textbox', { name: 'Prompt (expanded)' });
    expect(editorText(big)).toBe('Short');
    setEditorText(big, 'Short\n\n- and a list');
    expect(seen).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(seen).toHaveBeenLastCalledWith({ prompt: 'Short\n\n- and a list' });
    expect(editorText(screen.getByRole('textbox', { name: 'Prompt' }))).toBe('Short\n\n- and a list');
  });

  it('drops the edit on Cancel', async () => {
    const seen = vi.fn();
    render(<Form start={{ script: 'echo hi' }} seen={seen} />);
    await userEvent.click(screen.getByRole('button', { name: 'Expand Script' }));
    const dialog = screen.getByRole('dialog', { name: 'Script' });
    setEditorText(await within(dialog).findByRole('textbox', { name: 'Script (expanded)' }), 'rm -rf /');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(seen).not.toHaveBeenCalled();
    expect(editorText(screen.getByRole('textbox', { name: 'Script' }))).toBe('echo hi');
  });

  it('only shows the text when the form is read-only', async () => {
    render(<Form start={{ script: 'echo hi' }} seen={vi.fn()} readOnly />);
    await userEvent.click(screen.getByRole('button', { name: 'Expand Script' }));
    const dialog = screen.getByRole('dialog', { name: 'Script' });
    expect(within(dialog).queryByRole('button', { name: 'Done' })).toBeNull();
    await userEvent.click(within(dialog).getAllByRole('button', { name: 'Close' }).at(-1)!);
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

// PDT-469: an optional object that isn't set is an "Add …" button, not an open form with required fields.
describe('optional objects', () => {
  const CONFIRM = {
    type: 'object',
    $defs: {
      RunConfirm: {
        type: 'object',
        properties: { showIf: { anyOf: [{ $ref: '#/$defs/ShowIf' }, { type: 'null' }] }, text: { type: 'string', default: '' } },
      },
      ShowIf: { type: 'object', properties: { equals: true, field: { type: 'string' } }, required: ['field', 'equals'] },
    },
    properties: { confirm: { anyOf: [{ $ref: '#/$defs/RunConfirm' }, { type: 'null' }] } },
  };

  function ConfirmForm({ start, seen, readOnly }: { start: Record<string, unknown>; seen: (v: Record<string, unknown>) => void; readOnly?: boolean }) {
    const [value, setValue] = useState(start);
    return <SchemaForm schema={CONFIRM} value={value} readOnly={readOnly} onChange={(v) => { setValue(v); seen(v); }} />;
  }

  it('stay collapsed until added, then show their fields; Remove unsets them', async () => {
    const user = userEvent.setup();
    const seen = vi.fn();
    render(<ConfirmForm start={{}} seen={seen} />);
    expect(screen.queryByRole('textbox', { name: 'Confirm text' })).toBeNull();
    expect(screen.queryByText('Field')).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Add confirm' }));
    expect(seen).toHaveBeenLastCalledWith({ confirm: {} });
    expect(screen.getByRole('textbox', { name: 'Confirm text' })).toBeInTheDocument();
    // The nested optional Show if is collapsed too: no required fields yet.
    expect(screen.queryByRole('textbox', { name: 'Confirm show if field' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Add show if' }));
    expect(screen.getByRole('textbox', { name: 'Confirm show if field' })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Remove confirm' }));
    expect(seen.mock.lastCall![0]).not.toHaveProperty('confirm');
    expect(screen.getByRole('button', { name: 'Add confirm' })).toBeInTheDocument();
  });

  it('read-only shows "Not set" and no buttons', () => {
    render(<ConfirmForm start={{}} seen={vi.fn()} readOnly />);
    expect(screen.getByText('Not set')).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
  });
});
