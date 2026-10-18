import { describe, expect, it, vi } from 'vitest';
import { agentMenu } from './menus';

describe('agentMenu', () => {
  it('offers Open and Rename, and nothing that moves tabs', () => {
    const open = vi.fn();
    const rename = vi.fn();
    const entries = agentMenu(open, rename);
    expect(entries.map((e) => ('label' in e ? e.label : null))).toEqual(['Open', 'Rename']);
    (entries[0] as { onSelect: () => void }).onSelect();
    expect(open).toHaveBeenCalled();
  });

  it('leaves out Rename for an agent without a tab', () => {
    expect(agentMenu(() => {}).map((e) => ('label' in e ? e.label : null))).toEqual(['Open']);
  });
});
