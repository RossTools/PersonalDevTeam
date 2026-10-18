import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Textarea } from './Textarea';

describe('Textarea', () => {
  it('is a multi-line textbox with the native API', async () => {
    const onChange = vi.fn();
    render(<Textarea aria-label="Prompt" onChange={onChange} mono rows={4} />);
    const t = screen.getByRole('textbox', { name: 'Prompt' });
    expect(t.tagName).toBe('TEXTAREA');
    expect(t).toHaveAttribute('rows', '4');
    expect(t.className).toContain('font-mono');
    await userEvent.type(t, 'a{Shift>}{Enter}{/Shift}b');
    expect((t as HTMLTextAreaElement).value).toBe('a\nb');
    expect(onChange).toHaveBeenCalled();
  });
});
