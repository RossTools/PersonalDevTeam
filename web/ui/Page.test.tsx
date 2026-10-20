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
  it('is a region named by its heading, with description, actions and its rows in a card', () => {
    render(
      <Section label="Remote access" description="Reach this host over Tailscale." actions={<button type="button">Check again</button>}>
        <p>row one</p>
        <p>row two</p>
      </Section>,
    );
    const section = screen.getByRole('region', { name: 'Remote access' });
    const heading = within(section).getByRole('heading', { name: 'Remote access' });
    expect(heading.className).toContain('font-semibold');
    expect(section).toHaveTextContent('Reach this host over Tailscale.');
    expect(within(section).getByRole('button', { name: 'Check again' })).toBeInTheDocument();
    const card = section.querySelector('[data-slot="section-card"]')!;
    expect(card.className).toContain('rounded-lg');
    expect(card.className).toContain('bg-card');
    expect(card).toContainElement(screen.getByText('row one'));
    expect(screen.getByText('row one').parentElement!.className).toContain('divide-y');
  });

  it('puts the footer (a form\'s Save) at the bottom of the card', () => {
    render(<Section label="Runs" footer={<button type="submit">Save</button>}><p>Ports</p></Section>);
    const footer = screen.getByRole('region', { name: 'Runs' }).querySelector('[data-slot="section-footer"]')!;
    expect(footer).toContainElement(screen.getByRole('button', { name: 'Save' }));
    expect(footer.className).toContain('justify-end');
    expect(footer.className).toContain('border-t');
  });

  it('danger variant has a red heading and border', () => {
    render(<Section label="Danger zone" danger><p>Remove project</p></Section>);
    const section = screen.getByRole('region', { name: 'Danger zone' });
    expect(section.querySelector('[data-slot="section-card"]')!.className).toContain('border-destructive/40');
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

  it('stacked puts the label above a full-width control at every width', () => {
    render(<SettingsRow stacked label="Host overrides"><textarea aria-label="Host overrides" /></SettingsRow>);
    expect(screen.getByRole('group', { name: 'Host overrides' }).className).not.toContain('sm:flex-row');
  });
});
