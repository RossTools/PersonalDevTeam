import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { PromptBox } from './PromptBox';

function Box({ onSend, onStop }: { onSend: (t: string) => void; onStop?: () => void }) {
  const [text, setText] = useState('');
  return <PromptBox label="Message" value={text} onChange={setText} onSend={() => onSend(text)} onStop={onStop} toolbar={<span>chips</span>} meta="ctx" />;
}

describe('PromptBox', () => {
  it('sends only with text, through a round icon button', async () => {
    const onSend = vi.fn();
    render(<Box onSend={onSend} />);
    const send = screen.getByRole('button', { name: 'Send' });
    expect(send).toBeDisabled();
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'hi');
    await userEvent.click(send);
    expect(onSend).toHaveBeenCalledWith('hi');
    expect(screen.getByText('chips')).toBeInTheDocument();
  });

  it('shows Stop while something runs, and Send again once there is text', async () => {
    const onStop = vi.fn();
    render(<Box onSend={() => {}} onStop={onStop} />);
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(onStop).toHaveBeenCalled();
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'next');
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled();
  });
});
