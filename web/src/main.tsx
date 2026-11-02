import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClientProvider } from '@tanstack/react-query';
import { applyThemePref, ConfirmProvider, readThemePref, Toaster } from '@ui';
import { AppShell } from './shell/AppShell';
import { PairGate } from './pairing/PairGate';
import { createQueryClient } from './common/queryClient';
import { reloadOnStaleChunks } from './common/staleBuild';
import './index.css';

applyThemePref(readThemePref());
reloadOnStaleChunks();
const queryClient = createQueryClient();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ConfirmProvider>
        <PairGate>
          <AppShell />
        </PairGate>
        <Toaster />
      </ConfirmProvider>
    </QueryClientProvider>
  </StrictMode>,
);
