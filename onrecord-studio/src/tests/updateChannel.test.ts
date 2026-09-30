import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = join(__dirname, '..', '..');
const read = (path: string) => readFileSync(join(root, path), 'utf8');
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const json = (path: string) => JSON.parse(read(path)) as Record<string, any>;

describe('update channel', () => {
  it('links the updater only through the direct-license feature', () => {
    const cargo = read('src-tauri/Cargo.toml');
    expect(cargo).toMatch(/tauri-plugin-updater = \{[^}]*optional = true/);
    expect(cargo).toMatch(/tauri-plugin-process = \{[^}]*optional = true/);
    expect(cargo).toMatch(/^direct-license = \[[^\]]*dep:tauri-plugin-updater/m);
    expect(cargo).not.toMatch(/^app-store = \[[^\]]*updater/m);
    expect(read('src-tauri/src/lib.rs')).toMatch(/#\[cfg\(feature = "direct-license"\)\]\s+let builder = builder\s+\.plugin\(tauri_plugin_updater/);
  });

  it('grants the updater capability to the direct build only', () => {
    expect(json('src-tauri/tauri.conf.json').app.security.capabilities).toEqual(['main-window-capability']);
    expect(json('src-tauri/tauri.direct.conf.json').app.security.capabilities).toContain('direct-updater-capability');
    const capability = json('src-tauri/capabilities/direct.json');
    expect(capability.identifier).toBe('direct-updater-capability');
    expect(capability.permissions).toEqual(['updater:allow-check', 'updater:allow-download-and-install', 'process:allow-restart']);
    expect(JSON.stringify(json('src-tauri/capabilities/main.json'))).not.toMatch(/updater|process:/);
  });

  it('pins an https feed and a public key, with no private material', () => {
    const updater = json('src-tauri/tauri.direct.conf.json').plugins.updater;
    expect(updater.endpoints).toEqual(['https://updates.writerslogic.com/onrecord/updates/{{target}}/{{arch}}/{{current_version}}']);
    const key = Buffer.from(updater.pubkey, 'base64').toString('utf8');
    expect(key).toMatch(/^untrusted comment: minisign public key/);
    expect(key).not.toMatch(/secret key/i);
  });

  it('never lets update code read or write the license or account', () => {
    const dir = join(root, 'src/components/updates');
    for (const name of readdirSync(dir)) {
      const source = readFileSync(join(dir, name), 'utf8');
      expect(source, name).not.toMatch(/^import .*from '[^']*(licens|account|ipc|store)[^']*'|invokeCommand|entitlement/im);
    }
  });

  it('verifies a minisign signature and rejects tampered data or a foreign key', async () => {
    const modulePath = join(root, 'scripts/updater-signature-direct.mjs');
    const { verifyUpdaterSignature } = (await import(/* @vite-ignore */ modulePath)) as {
      verifyUpdaterSignature: (input: { pubkey: string; signature: string; data: Buffer }) => boolean;
    };
    const fixture = json('src/tests/fixtures/updater-signature.json') as { pubkey: string; signature: string };
    const data = Buffer.from('fixture payload');
    expect(verifyUpdaterSignature({ ...fixture, data })).toBe(true);
    expect(() => verifyUpdaterSignature({ ...fixture, data: Buffer.from('fixture payloae') })).toThrow(/does not verify/);
    const shipped = json('src-tauri/tauri.direct.conf.json').plugins.updater.pubkey as string;
    expect(() => verifyUpdaterSignature({ pubkey: shipped, signature: fixture.signature, data })).toThrow(/different key/);
  });
});
