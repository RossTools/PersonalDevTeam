import { useState, type ReactNode } from 'react';
import { Badge } from '@ui/Badge';
import { ChoiceCard, ChoiceCardGroup } from '@ui/ChoiceCard';
import { Disclosure } from '@ui/Disclosure';
import { ListHeading, SectionLabel } from '@ui/SectionLabel';
import { TabPanel, Tabs } from '@ui/Tabs';
import { IconButton } from '@ui/IconButton';
import { Plus } from 'lucide-react';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      <div className="flex flex-wrap items-start gap-3">{children}</div>
    </div>
  );
}

function TabsDemo() {
  const [tab, setTab] = useState('overview');
  return (
    <div className="flex h-36 w-full max-w-xl flex-col rounded-md border border-border">
      <Tabs
        aria-label="Ticket sections"
        value={tab}
        onChange={setTab}
        tabs={[
          { id: 'overview', label: 'Overview' },
          { id: 'conversation', label: 'Conversation' },
          { id: 'changes', label: <>Changes <span className="ml-1.5 text-2xs text-running">+12</span></> },
          { id: 'later', label: 'Later', disabled: true },
        ]}
      >
        <TabPanel id="overview" className="p-3 text-sm">The description, activity and details.</TabPanel>
        <TabPanel id="conversation" className="p-3 text-sm">The ticket's agent sessions.</TabPanel>
        <TabPanel id="changes" className="p-3 text-sm">The branch's diff and review.</TabPanel>
        <TabPanel id="later" className="p-3 text-sm">Never shown.</TabPanel>
      </Tabs>
    </div>
  );
}

function CardsDemo() {
  const [picked, setPicked] = useState('blank');
  const [time, setTime] = useState('09:00');
  const [agent, setAgent] = useState('claude');
  return (
    <div className="flex w-full flex-col gap-4">
      <div className="grid max-w-2xl grid-cols-1 gap-2 sm:grid-cols-3">
        <ChoiceCard dashed title="Start from scratch" description="An empty workflow." selected={picked === 'blank'} onClick={() => setPicked('blank')} />
        <ChoiceCard title="Ticket pipeline" description="Code, review and QA every ticket." selected={picked === 'pipeline'} onClick={() => setPicked('pipeline')}>
          <span className="flex flex-wrap gap-1.5"><Badge>On a ticket</Badge><Badge tone="running">Added</Badge></span>
        </ChoiceCard>
        <ChoiceCard title="Nightly digest" description="Plugin turned off." disabled />
      </div>
      <div className="grid w-64 grid-cols-2 gap-1.5">
        {[['09:00', 'Morning'], ['13:00', 'Afternoon'], ['17:00', 'Evening'], ['20:00', 'Night']].map(([t, l]) => (
          <ChoiceCard key={t} size="sm" title={<span className="tabular-nums">{t}</span>} description={l} selected={time === t} onClick={() => setTime(t)} />
        ))}
      </div>
      <ChoiceCardGroup
        aria-label="Default agent"
        size="sm"
        value={agent}
        onChange={setAgent}
        className="max-w-xl"
        options={[
          { value: 'claude', title: 'Claude Code', description: 'Found' },
          { value: 'codex', title: 'Codex', description: 'Found' },
          { value: 'devin', title: 'Devin', description: 'Not installed', disabled: true },
        ]}
      />
    </div>
  );
}

function render() {
  return (
    <div className="flex flex-col gap-6">
      <Row label="Tabs (a page's sections; arrows move, one Tab stop)">
        <TabsDemo />
      </Row>
      <Row label="Disclosure · inline (a muted line that opens what's under it)">
        <div className="flex w-full max-w-md flex-col gap-2">
          <Disclosure title="Show the comments" panelClassName="text-sm text-foreground">a.txt:2 — Make it TWO.</Disclosure>
          <Disclosure title="Advanced" defaultOpen className="text-sm" panelClassName="text-foreground">Use your own OAuth client.</Disclosure>
        </div>
      </Row>
      <Row label="Disclosure · row (a folding section, 44px on touch)">
        <div className="flex w-full max-w-sm flex-col divide-y divide-border rounded-md border border-border">
          <Disclosure variant="row" title="Details" summary="Ready · High · ui" panelClassName="px-2 pb-2 text-sm">Stage, priority, labels.</Disclosure>
          <Disclosure variant="row" title="Alpha" aria-label="Alpha" trailing={<Badge>ALP</Badge>} panelClassName="px-2 pb-2 text-sm">General · Runs · Agents</Disclosure>
        </div>
      </Row>
      <Row label="ChoiceCard · ChoiceCardGroup">
        <CardsDemo />
      </Row>
      <Row label="SectionLabel · ListHeading">
        <div className="flex w-64 flex-col rounded-md border border-border">
          <ListHeading trailing={<IconButton label="New note" size="sm" noTooltip className="size-5"><Plus aria-hidden /></IconButton>}>Notes</ListHeading>
          <div className="px-3 py-1 text-sm">A note</div>
          <ListHeading as="h3">Needs you <Badge size="sm" count={2} /></ListHeading>
          <div className="px-3 py-1 text-sm">An item</div>
          <div className="p-3"><SectionLabel>Links</SectionLabel></div>
        </div>
      </Row>
    </div>
  );
}

export const section = { title: 'Tabs, disclosures and choices', order: 15, render };
