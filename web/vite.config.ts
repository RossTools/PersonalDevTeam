import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@ui': fileURLToPath(new URL('./ui', import.meta.url)) } },
  // Dev server talks to the *dev* daemon (PDT_PROFILE=dev → port 7421).
  server: { proxy: { '/api': { target: 'http://127.0.0.1:7421', ws: true } } },
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'], css: false },
});
