import { defineConfig } from 'vitest/config';
import path from 'path';

// Most suites run without a DOM: they exercise pure logic (store reducers, offset math, URL/DID
// validation) rather than rendered components, so the default node environment is correct and
// keeps them dependency-free. src/tests/App.test.tsx opts itself into jsdom per-file.
export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify('0.0.0-test') },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  test: {
    // `.tsx` is included now that jsdom is a dev dependency. The default environment stays
    // `node`: only the file that needs a DOM opts in, via a `// @vitest-environment jsdom`
    // docblock, so the pure suites keep running without one.
    include: ['src/tests/**/*.test.{ts,tsx}'],
    environment: 'node',
  },
});
