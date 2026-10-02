import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { readFileSync } from 'fs';
import path from 'path';

// package.json is the version the release scripts check against Cargo.toml and tauri.conf.json.
const { version } = JSON.parse(readFileSync(path.resolve(__dirname, 'package.json'), 'utf8')) as { version: string };

// Set by the App Store and direct release scripts so Help describes only the build it ships in.
// Development leaves it unset and shows every topic.
const distribution = process.env.ONRECORD_DISTRIBUTION ?? '';
if (!['', 'app-store', 'direct'].includes(distribution)) {
  throw new Error(`ONRECORD_DISTRIBUTION must be app-store or direct, not ${distribution}.`);
}

// https://vitejs.dev
export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(version), __DISTRIBUTION__: JSON.stringify(distribution || null) },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  // Prevent Vite from obscuring native Rust compilation printouts
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  // Pack targets optimal for Tauri native runtime platforms
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: process.env.TAURI_ENV_PLATFORM == 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
