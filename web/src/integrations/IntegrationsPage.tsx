// Settings → Integrations (PDT-173): which of the integrations this PDT was built with this instance uses. A turned-off
// integration adds nothing (no connection kinds, workflow steps, PM tools or checks); its connections, project
// settings and workflows are kept for when it is turned on again. One this build lacks is listed, and can't be turned on.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Badge, Page, Section, Skeleton, Switch } from '@ui';
import { api } from '../api/client';
import type { IntegrationInfo } from '../api/gen/IntegrationInfo';

export const integrationsKey = ['integrations'] as const;

function Row({ i, pending, onChange }: { i: IntegrationInfo; pending: boolean; onChange: (on: boolean) => void }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-2 text-sm font-medium">
          {i.label}
          {!i.built && <Badge>Not in this build</Badge>}
        </span>
        {i.description && <span className="text-xs text-muted-foreground">{i.description}</span>}
      </div>
      <Switch aria-label={`Use ${i.label}`} checked={i.enabled} disabled={!i.built || pending} onChange={onChange} />
    </div>
  );
}

export function IntegrationsPage() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: integrationsKey, queryFn: api.listIntegrations });
  const set = useMutation({
    mutationFn: (enabled: string[]) => api.setIntegrations(enabled),
    onSuccess: (next) => {
      qc.setQueryData(integrationsKey, next);
      // Connection kinds, the node catalog, project settings and the personas' tools all follow what is on.
      void qc.invalidateQueries({ predicate: (q) => q.queryKey[0] !== integrationsKey[0] });
    },
  });
  const items = list.data ?? [];
  const toggle = (id: string, on: boolean) => {
    const enabled = items.filter((i) => (i.id === id ? on : i.enabled)).map((i) => i.id);
    set.mutate(enabled);
  };
  return (
    <Page
      title="Integrations"
      description="The services this PDT works with. Turn on the ones you use: each adds its kinds of connection, workflow steps and the PM's tools. Turning one off keeps its connections, project settings and workflows, which wait until it is on again."
    >
      {list.error && <Alert tone="error">Integrations could not be loaded: {list.error.message}</Alert>}
      {list.isLoading && <Skeleton label="Loading integrations" className="h-24" />}
      {!list.isLoading && !list.error && (
        <Section label="Integrations">
          {items.length
            ? items.map((i) => <Row key={i.id} i={i} pending={set.isPending} onChange={(on) => toggle(i.id, on)} />)
            : <p className="text-sm text-muted-foreground">This PDT was built without any.</p>}
        </Section>
      )}
      {set.error && <Alert tone="error">{set.error.message}</Alert>}
    </Page>
  );
}
