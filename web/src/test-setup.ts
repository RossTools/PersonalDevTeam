import '@testing-library/jest-dom/vitest';
import { cleanup, configure } from '@testing-library/react';
import { afterEach } from 'vitest';

// Vitest globals are off, so RTL's automatic cleanup does not register itself.
afterEach(() => cleanup());

// findBy* / waitFor give up after this long. Lazy chunks and a slow CI runner (Windows) can take over the default 1 s.
configure({ asyncUtilTimeout: process.env.CI ? 10_000 : 3_000 });

// jsdom gaps that React Aria and Sonner touch (skipped in node-environment tests such as lint.test.ts).
if (typeof window !== 'undefined') {
  Element.prototype.setPointerCapture ??= () => {};
  Element.prototype.releasePointerCapture ??= () => {};
  // React Flow measures nodes with ResizeObserver.
  window.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  window.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

// The app keeps what is on screen in the path (PDT-111): every test starts at `/`.
afterEach(() => {
  if (typeof window !== 'undefined') window.history.replaceState(null, '', '/');
});
