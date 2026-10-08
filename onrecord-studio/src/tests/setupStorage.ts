import { afterEach } from 'vitest';

// Node 26 defines a flag-gated `localStorage` global whose getter returns undefined, and it
// shadows the Storage jsdom installs; Node 24 (CI) leaves jsdom's in place. Give every jsdom suite
// a working Storage on either runtime, and empty it after each test so a remembered choice (a chart's
// table toggle, the theme) never leaks into the next test.
if (typeof window !== 'undefined') {
  if (globalThis.localStorage === undefined) {
    const entries = new Map<string, string>();
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      value: {
        getItem: (key: string) => entries.get(key) ?? null,
        setItem: (key: string, value: string) => void entries.set(key, String(value)),
        removeItem: (key: string) => void entries.delete(key),
        clear: () => entries.clear(),
        key: (index: number) => [...entries.keys()][index] ?? null,
        get length() {
          return entries.size;
        },
      },
    });
  }
  afterEach(() => {
    globalThis.localStorage?.clear();
  });
}
