import { useState, type ReactNode } from 'react';
import { Folder, Link2, Maximize2, Monitor, Moon, Play, Plus, Settings, Square, Sun, Trash2, X } from 'lucide-react';
import { Button, type ButtonVariant } from '@ui/Button';
import { IconButton } from '@ui/IconButton';
import { Input } from '@ui/Input';
import { Textarea } from '@ui/Textarea';
import { Select } from '@ui/Select';
import { SuggestInput } from '@ui/SuggestInput';
import { Field } from '@ui/Field';
import { Checkbox } from '@ui/Checkbox';
import { CheckboxPicks } from '@ui/CheckboxPicks';
import { Switch } from '@ui/Switch';
import { SegmentedControl } from '@ui/SegmentedControl';
import { Kbd } from '@ui/Kbd';
import { Link } from '@ui/Link';
import { Spinner } from '@ui/Spinner';
import { Skeleton } from '@ui/Skeleton';
import { Tooltip } from '@ui/Tooltip';
import { SectionLabel } from '@ui/SectionLabel';

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel as="div">{label}</SectionLabel>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}

const variants: ButtonVariant[] = ['primary', 'secondary', 'ghost', 'danger'];

function PendingDemo() {
  const [pending, setPending] = useState(false);
  return (
    <Button
      variant="primary"
      pending={pending}
      onClick={() => {
        setPending(true);
        setTimeout(() => setPending(false), 1500);
      }}
    >
      Save changes
    </Button>
  );
}

function ToggleDemo() {
  const [max, setMax] = useState(false);
  return (
    <IconButton label={max ? 'Restore panel' : 'Maximize panel'} pressed={max} onClick={() => setMax((m) => !m)}>
      <Maximize2 />
    </IconButton>
  );
}

function FormDemo() {
  const [name, setName] = useState('Personal Dev Team');
  const [key, setKey] = useState('PD');
  const [agent, setAgent] = useState('claude');
  const [prompt, setPrompt] = useState('');
  const [remote, setRemote] = useState(true);
  const [skip, setSkip] = useState(false);
  const [del, setDel] = useState(false);
  const [source, setSource] = useState('folder');
  const [theme, setTheme] = useState('system');
  const [picked, setPicked] = useState<string[]>(['pdt']);
  return (
    <div className="grid max-w-xl gap-4">
      <CheckboxPicks
        label="Projects"
        hint="Every project when none is ticked."
        options={[{ id: 'pdt', name: 'Personal Dev Team' }, { id: 'site', name: 'Website' }, { id: 'notes', name: 'Notes' }]}
        value={picked}
        onChange={setPicked}
      />
      <SegmentedControl
        aria-label="Source"
        options={[
          { value: 'folder', label: 'Folder', icon: <Folder /> },
          { value: 'clone', label: 'Clone URL', icon: <Link2 /> },
        ]}
        value={source}
        onChange={setSource}
      />
      {/* Vertical, icons only: the collapsed side rail's theme switch. */}
      <SegmentedControl
        aria-label="Theme (vertical)"
        size="sm"
        orientation="vertical"
        options={[
          { value: 'system', label: null, icon: <Monitor />, 'aria-label': 'System' },
          { value: 'light', label: null, icon: <Sun />, 'aria-label': 'Light' },
          { value: 'dark', label: null, icon: <Moon />, 'aria-label': 'Dark' },
        ]}
        value={theme}
        onChange={setTheme}
      />
      <Field label="Name" hint="Shown in the sidebar and tabs.">
        <Input value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Key" error={key.length < 3 ? 'Use 3–6 capital letters.' : undefined}>
        <Input mono value={key} onChange={(e) => setKey(e.target.value.toUpperCase())} />
      </Field>
      <Field label="Agent">
        <Select
          value={agent}
          onChange={setAgent}
          options={[
            { value: 'claude', label: 'Claude Code' },
            { value: 'codex', label: 'Codex' },
            { value: 'shell', label: 'Shell (disabled)', disabled: true },
          ]}
        />
      </Field>
      <Field label="Prompt" hint="Shift+Enter for a new line.">
        <Textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="Describe the change…" />
      </Field>
      <div className="flex flex-col divide-y divide-border rounded-md border border-border px-3">
        <Field label="Remote access" hint="Serve this host over Tailscale." inline>
          <Switch checked={remote} onChange={setRemote} />
        </Field>
        <Field label="Theme" inline>
          <SegmentedControl
            aria-label="Theme"
            size="sm"
            options={[
              { value: 'system', label: 'System' },
              { value: 'light', label: 'Light' },
              { value: 'dark', label: 'Dark' },
            ]}
            value={theme}
            onChange={setTheme}
          />
        </Field>
        <Field label="Run ports" inline error="End must be after start.">
          <div className="flex gap-1">
            <Input size="sm" className="w-20" aria-label="Start" defaultValue="8400" />
            <Input size="sm" className="w-20" aria-label="End" defaultValue="8300" aria-invalid />
          </div>
        </Field>
      </div>
      <div className="flex flex-wrap gap-4">
        <Switch label="Skip permissions" checked={skip} onChange={setSkip} />
        <Switch label="Small switch" size="sm" defaultChecked />
        <Switch label="Disabled" disabled />
      </div>
      <div className="flex flex-wrap gap-4">
        <Checkbox checked={del} onChange={setDel}>
          Also delete the branch
        </Checkbox>
        <Checkbox label="Indeterminate" indeterminate />
        <Checkbox label="Disabled" disabled />
        <Checkbox shape="round" label="Round (todos)" />
        <Checkbox shape="round" label="Round, ticked" defaultChecked />
        <Checkbox shape="round" danger label="Round, overdue" />
      </div>
    </div>
  );
}

function render() {
  return (
    <div className="flex flex-col gap-5">
      <Row label="Button · md (28px)">
        {variants.map((v) => (
          <Button key={v} variant={v}>
            {v[0].toUpperCase() + v.slice(1)}
          </Button>
        ))}
        <Button variant="primary">
          <Plus /> New session
        </Button>
        <Button disabled>Disabled</Button>
        <PendingDemo />
        <Button pending>Pending</Button>
      </Row>
      <Row label="Button · sm (24px)">
        {variants.map((v) => (
          <Button key={v} variant={v} size="sm">
            {v[0].toUpperCase() + v.slice(1)}
          </Button>
        ))}
        <Button size="sm" variant="ghost">
          <Play /> Run
        </Button>
        <Button size="sm" variant="danger">
          <Trash2 /> Remove worktree
        </Button>
      </Row>
      <Row label="Button · lg (48px on every pointer: a phone screen's main actions)">
        <div className="flex w-full max-w-sm gap-2">
          <Button size="lg" className="min-w-0 flex-1">Comment…</Button>
          <Button size="lg" variant="primary" className="min-w-0 flex-1">Start</Button>
        </div>
      </Row>
      <Row label="IconButton (label = name + tooltip)">
        <IconButton label="Close tab">
          <X />
        </IconButton>
        <IconButton label="New tab" shortcut={<Kbd>T</Kbd>}>
          <Plus />
        </IconButton>
        <IconButton label="Settings" variant="secondary">
          <Settings />
        </IconButton>
        <IconButton label="Stop" variant="danger">
          <Square />
        </IconButton>
        <IconButton label="Small close" size="sm">
          <X />
        </IconButton>
        <ToggleDemo />
        <IconButton label="Disabled" disabled>
          <Trash2 />
        </IconButton>
        <IconButton label="Pending" pending>
          <Play />
        </IconButton>
      </Row>
      <Row label="Tooltip">
        <Tooltip content="Runs pnpm test in the checkout">
          <Button variant="ghost">Hover or focus me</Button>
        </Tooltip>
        <Button title="title= becomes a Tooltip">With title</Button>
      </Row>
      <Row label="Input · Select (sizes match Button)">
        <Input aria-label="Medium input" placeholder="md input" className="w-40" />
        <Input aria-label="Small input" size="sm" placeholder="sm input" className="w-40" />
        <Select
          aria-label="Branch"
          value={null}
          onChange={() => {}}
          placeholder="Pick a branch"
          options={[
            { value: 'main', label: 'main' },
            { value: 'feat', label: 'phase-3c-1-ui' },
          ]}
        />
        <Select aria-label="Small select" size="sm" value="a" onChange={() => {}} options={[{ value: 'a', label: 'Small' }]} />
        <Select
          aria-label="Searchable select"
          searchable
          value="main"
          onChange={() => {}}
          options={['main', 'phase-3c-1-ui', 'pdt-PDT-92-issues', 'release/1.2'].map((b) => ({ value: b, label: b }))}
        />
        <Input aria-label="Disabled input" disabled value="disabled" className="w-32" readOnly />
      </Row>
      <Row label="SuggestInput (pick a suggestion with one click, or type and press Enter)">
        <SuggestDemo />
      </Row>
      <Row label="Field · Checkbox · Switch · SegmentedControl">
        <FormDemo />
      </Row>
      <Row label="Kbd">
        <Kbd>Esc</Kbd>
        <Kbd>⌘K</Kbd>
        <Kbd keys={['Ctrl', ']']} />
        <span className="text-xs text-muted-foreground">
          then <Kbd>t</Kbd> new tab
        </span>
      </Row>
      <Row label="Link">
        <Link onClick={() => {}}>Open settings</Link>
        <Link href="#gallery">Internal href</Link>
        <Link href="https://github.com/example-org/PersonalDevTeam" external>
          GitHub
        </Link>
      </Row>
      <Row label="Spinner">
        <Spinner />
        <Spinner className="size-5" label="Cloning" />
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Spinner decorative /> Checking for updates…
        </span>
      </Row>
      <Row label="Skeleton">
        <div className="w-64">
          <Skeleton lines={3} />
        </div>
        <Skeleton className="h-16 w-40" />
      </Row>
    </div>
  );
}

export const section = { title: 'Controls', order: 10, render };

function SuggestDemo() {
  const [added, setAdded] = useState<string[]>([]);
  return (
    <>
      <SuggestInput aria-label="Add label" size="sm" placeholder="Add label…" suggestions={['backend', 'bug', 'docs', 'perf', 'ui']} onCommit={(v) => setAdded((a) => [...a, v])} className="w-40" />
      <span className="text-xs text-muted-foreground">{added.length > 0 ? `Added: ${added.join(', ')}` : 'Nothing added yet'}</span>
    </>
  );
}
