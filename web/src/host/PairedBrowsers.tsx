import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Input, Section, SettingsRow, useConfirm } from '@ui';
import { api } from '../api/client';
import type { PairLink } from '../api/gen/PairLink';
import { copyToClipboard } from '../common/clipboard';
import { useAction } from '../common/useAction';
import { PAIRING_KEY } from '../pairing/queries';

/** Settings › This host › Browsers (PDT-401): pair another browser, or unpair them all. */
export function PairedBrowsers() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const pairing = useQuery({ queryKey: PAIRING_KEY, queryFn: api.pairing });
  const host = useQuery({ queryKey: ['host-settings'], queryFn: api.hostSettings });
  const [link, setLink] = useState<PairLink | null>(null);
  const [make, making] = useAction(async () => setLink(await api.newPairLink()));
  const [unpair, unpairing] = useAction(async () => {
    const { ok } = await confirm({
      title: 'Unpair every browser?',
      body: 'Every browser, this one too, needs pairing again before it can make the changes only you may make.',
      action: 'Unpair all',
      danger: true,
    });
    if (!ok) return;
    await api.unpairAll();
    setLink(null);
    void qc.invalidateQueries({ queryKey: PAIRING_KEY });
  });
  const n = pairing.data?.pairedBrowsers;
  // On the tailnet address too, when remote access is on and this page isn't already there: the link for a phone.
  const remote = host.data?.remote.active ? host.data.remote.url?.replace(/\/$/, '') : undefined;
  const urls = link
    ? [
        { label: 'Pairing link', url: `${location.origin}${link.path}` },
        ...(remote && remote !== location.origin ? [{ label: 'Pairing link on the tailnet', url: `${remote}${link.path}` }] : []),
      ]
    : [];
  return (
    <Section
      label="Browsers"
      description={`Approvals and the other changes only you may make come only from a browser you've paired.${n === undefined ? '' : ` ${n} ${n === 1 ? 'browser is' : 'browsers are'} paired.`}`}
    >
      <SettingsRow label="Pair another browser" description="A link that pairs the browser that opens it (a phone, too, on the tailnet address). It works once, within 5 minutes.">
        <Button size="sm" pending={making} onClick={() => void make()}>New link</Button>
      </SettingsRow>
      {urls.map(({ label, url }) => (
        <div key={label} className="flex items-center gap-2">
          <Input aria-label={label} readOnly value={url} className="min-w-0 flex-1 font-mono text-xs" />
          <Button size="sm" aria-label={`Copy ${label.toLowerCase()}`} onClick={() => void copyToClipboard(url, 'Link copied')}>Copy</Button>
        </div>
      ))}
      <SettingsRow label="Unpair all browsers" description="Every browser, this one too, needs pairing again.">
        <Button size="sm" variant="danger" pending={unpairing} onClick={() => void unpair()}>Unpair all…</Button>
      </SettingsRow>
    </Section>
  );
}
