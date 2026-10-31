import { useId, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Section, Select, SettingsRow } from '@ui';
import { api } from '../api/client';
import type { Project } from '../api/gen/Project';
import type { ProjectDocumentSettings } from '../api/gen/ProjectDocumentSettings';
import { SaveStatus, useSaveState } from '../common/AutoSave';
import { FolderField } from '../common/FolderField';
import { ProjectDocumentsBody } from './LinkedDocuments';
import { documentKeys, useDocumentProviders } from './queries';

/** "Follow the host" in a provider picker. */
const HOST = 'host';

/**
 * A project's Documents section (PDT-314): the provider its new documents go to and its local documents folder (each
 * blank: the host's), then the documents linked to it.
 */
export function ProjectDocuments({ project }: { project: Project }) {
  const qc = useQueryClient();
  const key = ['project-document-settings', project.id];
  const settings = useQuery({ queryKey: key, queryFn: () => api.projectDocumentSettings(project.id) });
  const hostView = useDocumentProviders();
  const [state, track] = useSaveState();
  const [folder, setFolder] = useState<string | null>(null);
  const providerId = useId();
  const s = settings.data;
  const save = async (patch: Partial<ProjectDocumentSettings>) => {
    const saved = await api.putProjectDocumentSettings(project.id, { provider: s?.provider ?? null, folder: s?.folder ?? null, ...patch });
    qc.setQueryData(key, saved);
    void qc.invalidateQueries({ queryKey: documentKeys.providers(project.id) });
    return saved;
  };
  const providers = hostView.data?.providers ?? [];
  const hostDefault = providers.find((p) => p.id === hostView.data?.default)?.label;
  const commitFolder = (value: string) => {
    if (value.trim() === (s?.folder ?? '')) return setFolder(null);
    void track(save({ folder: value.trim() || null })).then((ok) => { if (ok) setFolder(null); });
  };
  return (
    <ProjectDocumentsBody
      projectId={project.id}
      wrap={(actions, body) => (
        <Section label="Documents" description="Docs and sheets linked to this project, and where its new ones go." actions={actions}>
          {settings.isError && <Alert tone="error">Document settings could not be loaded: {settings.error.message}</Alert>}
          <SettingsRow label="Default provider" htmlFor={providerId} description="New documents in this project go here.">
            <SaveStatus state={state} />
            <Select
              id={providerId}
              aria-label="Default provider"
              className="w-56 max-w-full"
              options={[{ value: HOST, label: hostDefault ? `This host's (${hostDefault})` : "This host's" }, ...providers.map((p) => ({ value: p.id, label: p.label }))]}
              value={s?.provider ?? HOST}
              disabled={!s}
              onChange={(provider) => void track(save({ provider: provider === HOST ? null : provider }))}
            />
          </SettingsRow>
          <SettingsRow stacked label="Local documents folder" description="Where documents kept on this machine go. Blank: this host's folder.">
            <FolderField
              aria-label="Local documents folder"
              title="Local documents folder"
              placeholder={hostView.data?.folder ?? ''}
              start={s?.folder ?? hostView.data?.folder ?? ''}
              value={folder ?? s?.folder ?? ''}
              onChange={setFolder}
              onCommit={commitFolder}
            />
          </SettingsRow>
          <div className="flex flex-col gap-2 py-2">{body}</div>
        </Section>
      )}
    />
  );
}
