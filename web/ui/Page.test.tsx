import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Page, Section, SettingsRow } from './Page';

describe('Page', () => {
  it('is an article named by its title, with a description, and scrolls', () => {
    const { container } = render(
      <Page title="This host" description="Settings for this machine.">
        <p>body</p>
      </Page>,
    );
    const page = screen.getByRole('article', { name: 'This host' });
    expect(within(page).getByRole('heading', { name: 'This host' })).toBeInTheDocument();
    expect(page).toHaveTextContent('Settings for this machine.');
    expect(page.className).toContain('max-w-3xl');
    expect((container.firstChild as HTMLElement).className).toContain('overflow-auto');
  });
});

describe('Section', () => {
  it('is a region named by its label, with description and actions', () => {
    render(
      <Section label="Remote access" description="Reach this host over Tailscale." actions={<button type="button">Save</button>}>
        <p>rows</p>
      </Section>,
    );
    const section = screen.getByRole('region', { name: 'Remote access' });
    const heading = within(section).getByRole('heading', { name: 'Remote access' });
    expect(heading.className).toContain('text-2xs');
    expect(heading.className).toContain('uppercase');
    expect(heading.className).toContain('text-muted-foreground');
    expect(section).toHaveTextContent('Reach this host over Tailscale.');
    expect(within(section).getByRole('button', { name: 'Save' })).toBeInTheDocument();
  });

  it('danger variant has a soft destructive border and red label', () => {
    render(<Section label="Danger zone" danger><p>Remove project</p></Section>);
    const section = screen.getByRole('region', { name: 'Danger zone' });
    expect(section.className).toContain('border-destructive/30');
    expect(within(section).getByRole('heading').className).toContain('text-destructive');
  });
});

describe('SettingsRow', () => {
  it('labels its control with htmlFor', () => {
    render(
      <SettingsRow label="Owner login" description="Tailscale login allowed in." htmlFor="owner">
        <input id="owner" />
      </SettingsRow>,
    );
    expect(screen.getByRole('textbox', { name: 'Owner login' })).toBeInTheDocument();
    const group = screen.getByRole('group', { name: 'Owner login' });
    expect(group).toHaveAccessibleDescription('Tailscale login allowed in.');
  });

  it('stacks on phone and goes side by side from sm', () => {
    render(<SettingsRow label="Update policy"><select aria-label="Update policy" /></SettingsRow>);
    const group = screen.getByRole('group', { name: 'Update policy' });
    expect(group.className).toContain('flex-col');
    expect(group.className).toContain('sm:flex-row');
  });
});
