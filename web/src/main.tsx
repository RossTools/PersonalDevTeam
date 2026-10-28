import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { applyThemePref, ConfirmProvider, readThemePref, Toaster } from '@ui';
import { AppShell } from './shell/AppShell';
import { reloadOnStaleChunks } from './common/staleBuild';
import './index.css';

applyThemePref(readThemePref());
reloadOnStaleChunks();
const queryClient = new QueryClient();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ConfirmProvider>
        <AppShell />
        <Toaster />
      </ConfirmProvider>
    </QueryClientProvider>
  </StrictMode>,
);
