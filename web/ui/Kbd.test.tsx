import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Kbd } from './Kbd';

describe('Kbd', () => {
  it('renders one key as <kbd>', () => {
    render(<Kbd>Esc</Kbd>);
    expect(screen.getByText('Esc').tagName).toBe('KBD');
  });

  it('renders a sequence as separate keys', () => {
    const { container } = render(<Kbd keys={['Ctrl', ']']} />);
    const keys = [...container.querySelectorAll('[data-slot=kbd]')].map((k) => k.textContent);
    expect(keys).toEqual(['Ctrl', ']']);
  });
});
