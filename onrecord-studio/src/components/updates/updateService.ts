import type { Update } from '@tauri-apps/plugin-updater';

/** What the prompt needs from a pending update; the plugin's `Update` satisfies it. */
export interface PendingUpdate {
  version: string;
  notes: string;
  install: () => Promise<void>;
}

export const NOTES_LIMIT = 400;

// An unregistered plugin (App Store build) and a command outside the capability both mean this
// binary has no updater. That is the intended state there, not a failure to report.
const UNAVAILABLE = /not allowed|Plugin not found|Command .* not found/i;

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function describeUpdateError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'The update check failed without a usable error message.';
}

export function shortNotes(body: string | undefined): string {
  const notes = (body ?? '').trim();
  return notes.length > NOTES_LIMIT ? `${notes.slice(0, NOTES_LIMIT).trimEnd()}...` : notes;
}

/** Resolves null when the app is current, or when this build has no updater. */
export async function checkForUpdate(): Promise<PendingUpdate | null> {
  if (!isTauriRuntime()) return null;
  let update: Update | null;
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    update = await check();
  } catch (error) {
    if (UNAVAILABLE.test(describeUpdateError(error))) return null;
    throw error;
  }
  if (!update) return null;
  return {
    version: update.version,
    notes: shortNotes(update.body),
    // Replaces the binary only; no local state is read or written here.
    install: async () => {
      await update.downloadAndInstall();
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    },
  };
}
