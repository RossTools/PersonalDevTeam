import { defineConfig } from 'vitest/config';
import type { Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import optimizeLocales from '@react-aria/optimize-locales-plugin';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

/** The app's top-level paths (web/src/shell/route.ts). */
const APP_PATHS = /^\/(pm|sessions|inbox|tickets|notes|workflows|personas|files|settings)(\/|\?|$)/;

/**
 * Serves index.html for the app's paths, as pdtd does (PDT-111): Vite's own fallback would treat `/files/src/main.ts`
 * as a module to transform.
 */
const appPathFallback: Plugin = {
  name: 'pdt-app-path-fallback',
  configureServer(server) {
    server.middlewares.use((req, _res, next) => {
      if (req.method === 'GET' && req.url && APP_PATHS.test(req.url) && req.headers.accept?.includes('text/html')) req.url = '/';
      next();
    });
  },
};

/**
 * Names the build in index.html, `<meta name="web-build" content="…">` (12 hex digits of the page's hash), at build
 * time only. pdtd's /api/health reports the same id, and the page compares that with its own (useReloadOnNewBuild),
 * so a tab restored from a phone's background still knows which build its code came from.
 */
const webBuildMeta: Plugin = {
  name: 'pdt-web-build-meta',
  apply: 'build',
  transformIndexHtml: {
    order: 'post',
    handler(html) {
      const id = createHash('sha256').update(html).digest('hex').slice(0, 12);
      return [{ tag: 'meta', attrs: { name: 'web-build', content: id }, injectTo: 'head-prepend' }];
    },
  },
};

export default defineConfig({
  // React Aria ships strings for ~30 languages; PDT's UI is English only.
  plugins: [appPathFallback, webBuildMeta, react(), tailwindcss(), { ...optimizeLocales.vite({ locales: ['en-US'] }), enforce: 'pre' }],
  resolve: { alias: { '@ui': fileURLToPath(new URL('./ui', import.meta.url)) } },
  // Dev server talks to the *dev* daemon (PDT_PROFILE=dev → port 7421).
  server: { proxy: { '/api': { target: 'http://127.0.0.1:7421', ws: true } } },
  // CI's Windows runner is several times slower than a dev machine: tests there get more time (src/test-setup.ts
  // does the same for Testing Library's waits).
  test: { environment: 'jsdom', setupFiles: ['./src/test-setup.ts'], css: false, unstubGlobals: true, restoreMocks: true, testTimeout: process.env.CI ? 30_000 : 10_000 },
});
