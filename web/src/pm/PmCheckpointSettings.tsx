import { Checkbox, SettingsRow, Switch } from '@ui';
import { api } from '../api/client';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import { AutoSaveRow, AutoSaveText } from '../common/AutoSave';
import { BuiltinSettingsSection, ChatChannelRow, type PutInputs } from '../common/BuiltinSettings';

const PM_CHECKPOINT_KEY = ['pm-checkpoint'];

const DAYS = [
  { id: 'mon', label: 'Mon' }, { id: 'tue', label: 'Tue' }, { id: 'wed', label: 'Wed' }, { id: 'thu', label: 'Thu' },
  { id: 'fri', label: 'Fri' }, { id: 'sat', label: 'Sat' }, { id: 'sun', label: 'Sun' },
];

/**
 * Settings → This host → PM → Daily checkpoint (PDT-384): the PM daily checkpoint built-in's inputs: a time of day
 * (else each hour), the days, and a chat channel to post the daily update to.
 */
export function PmCheckpointSettings() {
  return (
    <BuiltinSettingsSection
      calls={api.pmCheckpoint}
      queryKey={PM_CHECKPOINT_KEY}
      label="Daily checkpoint"
      description="Each PM conversation you use gets what happened since its last update once a day, and writes a journal checkpoint."
      follows="The daily checkpoint follows"
      off="PM conversations get no daily update."
    >
      {(v, put) => <Inputs v={v} put={put} />}
    </BuiltinSettingsSection>
  );
}

function Inputs({ v, put }: { v: BuiltinSettingsView; put: PutInputs }) {
  const atTime = v.values.atTime === true;
  const days = Array.isArray(v.values.days) ? (v.values.days as string[]) : [];
  const toggle = (day: string, on: boolean) => DAYS.map((d) => d.id).filter((d) => (d === day ? on : days.includes(d)));
  return (
    <>
      <AutoSaveRow label="At a set time" description={atTime ? 'Once a day, at the time below.' : 'Off: each hour, to a conversation whose last update was a day or more ago.'}>
        {(track) => <Switch aria-label="At a set time" checked={atTime} onChange={(on) => void track(put({ atTime: on }))} />}
      </AutoSaveRow>
      {atTime && (
        <SettingsRow label="Time" description="HH:MM, in this computer’s time.">
          <AutoSaveText aria-label="Time" className="w-24" value={typeof v.values.time === 'string' ? v.values.time : ''} onSave={(t) => put({ time: t })} />
        </SettingsRow>
      )}
      <AutoSaveRow label="Days" description="The days updates are sent on.">
        {(track) => (
          <span className="flex flex-wrap gap-3">
            {DAYS.map((d) => (
              <Checkbox key={d.id} label={d.label} checked={days.includes(d.id)}
                disabled={days.length === 1 && days.includes(d.id)}
                onChange={(on) => void track(put({ days: toggle(d.id, on) }))} />
            ))}
          </span>
        )}
      </AutoSaveRow>
      {v.channels.length > 0 && <ChatChannelRow v={v} id="chat" put={put} what="Post the daily update" />}
    </>
  );
}
