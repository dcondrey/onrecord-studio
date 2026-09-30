import { isTauri } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';

export type OpenExternalResult = { ok: true } | { ok: false; reason: string };

/**
 * The single place external links leave the app. Only https URLs are opened; inside the app the
 * system browser is used through the opener plugin, whose capability scope allows just the
 * checkout, billing and site hosts. The address is also shown to the user so a refusal is never
 * a dead end.
 */
export async function openExternalUrl(url: string): Promise<OpenExternalResult> {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: 'The checkout address is not a valid URL.' };
  }
  if (parsed.protocol !== 'https:') {
    return { ok: false, reason: 'Only secure (https) checkout addresses are opened.' };
  }
  if (isTauri()) {
    try {
      await openUrl(parsed.toString());
      return { ok: true };
    } catch {
      return { ok: false, reason: 'The system browser could not be opened.' };
    }
  }
  const opened = window.open(parsed.toString(), '_blank', 'noopener,noreferrer');
  return opened ? { ok: true } : { ok: false, reason: 'The browser blocked the new window.' };
}
