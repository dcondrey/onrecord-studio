/** The package.json version, injected at build time (vite.config.ts and vitest.config.ts). */
declare const __APP_VERSION__: string;
/** Which release this bundle is, from ONRECORD_DISTRIBUTION at build time; null in development. */
declare const __DISTRIBUTION__: 'app-store' | 'direct' | null;
