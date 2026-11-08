import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PreviewBox } from './PreviewBox';

describe('PreviewBox', () => {
  it('holds its content, as a pre when asked', () => {
    const { container } = render(<><PreviewBox>A ticket</PreviewBox><PreviewBox as="pre">{'ls\n  -la'}</PreviewBox></>);
    expect(screen.getByText('A ticket').tagName).toBe('DIV');
    expect(container.querySelector('pre')?.textContent).toBe('ls\n  -la');
  });
});
