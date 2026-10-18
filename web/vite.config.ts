import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import optimizeLocales from '@react-aria/optimize-locales-plugin';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  // React Aria ships strings for ~30 languages; PDT's UI is English only.
  plugins: [react(), tailwindcss(), { ...optimizeLocales.vite({ locales: ['en-US'] }), enforce: 'pre' }],
  resolve: { alias: { '@ui': fileURLToPath(new URL('./ui', import.meta.url)) } },
  // Dev server talks to the *dev* daemon (PDT_PROFILE=dev → port 7421).
  server: { proxy: { '/api': { target: 'http://127.0.0.1:7421', ws: true } } },
  // CI's Windows runner is several times slower than a dev machine: tests there get more time (src/test-setup.ts
  // does the same for Testing Library's waits).
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'], css: false, testTimeout: process.env.CI ? 30_000 : 10_000 },
});
