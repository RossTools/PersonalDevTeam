// An agent's `request_secret` (PDT-158): the user types the value into this masked field, never into the chat. It
// goes to the keychain; the agent hears only that it is stored.
import { useState } from 'react';
import { Button, Field, Input } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';

const MIN_VALUE = 6;

export function SecretRequest({ item, busy, onSend }: { item: InboxItem; busy: boolean; onSend: (value: string) => Promise<void> }) {
  const [value, setValue] = useState('');
  const name = typeof item.body.name === 'string' ? item.body.name : 'secret';
  const purpose = typeof item.body.purpose === 'string' ? item.body.purpose : '';
  const ok = value.length >= MIN_VALUE;
  const send = async () => {
    if (!ok || busy) return;
    await onSend(value);
    setValue('');
  };
  return (
    <section aria-label="Secret request" className="flex flex-col gap-3">
      <p className="text-sm">
        {typeof item.body.sessionTitle === 'string' ? item.body.sessionTitle : 'An agent'} asks for the secret <code>{name}</code>
        {purpose ? `: ${purpose}` : '.'} It is kept in this machine's keychain; the agent can use it but never sees it.
        Never paste it into the chat.
      </p>
      <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); void send(); }}>
        <Field label={name} hint={`At least ${MIN_VALUE} characters. You can change who may use it in Settings → Secrets.`}>
          <Input aria-label={`Value of ${name}`} type="password" autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} />
        </Field>
        <div>
          <Button type="submit" variant="primary" pending={busy} disabled={!ok}>Store secret</Button>
        </div>
      </form>
    </section>
  );
}
