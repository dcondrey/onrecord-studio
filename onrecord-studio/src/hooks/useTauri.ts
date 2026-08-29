import { useState, useCallback, useRef } from 'react';
import { invokeCommand, type NativeCommandMap, type NativeCommandName } from '../ipc';
import { useAppStore } from '../store/useAppStore';

export function useTauri() {
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const activeCommands = useRef(0);
  const appendSyncLog = useAppStore((s) => s.appendSyncLog);

  /**
   * Safe wrapper for invoking native asynchronous Rust backend commands.
   */
  const executeNativeCommand = useCallback(async <Command extends NativeCommandName>(
    commandName: Command,
    args: NativeCommandMap[Command]['args'],
    options: { signal?: AbortSignal; maximumAttempts?: number } = {},
  ): Promise<NativeCommandMap[Command]['result'] | null> => {
    activeCommands.current += 1;
    setIsLoading(true);
    setError(null);

    try {
      const maximumAttempts = Math.max(
        1,
        Math.min(options.maximumAttempts ?? defaultAttempts(commandName), 4),
      );
      for (let attempt = 1; attempt <= maximumAttempts; attempt += 1) {
        if (options.signal?.aborted) return null;
        try {
          const result = await invokeCommand(commandName, args);
          return options.signal?.aborted ? null : result;
        } catch (reason) {
          if (options.signal?.aborted) return null;
          if (attempt === maximumAttempts) throw reason;
          await abortableDelay(150 * 2 ** (attempt - 1), options.signal);
        }
      }
      return null;
    } catch (err: unknown) {
      if (options.signal?.aborted || isAbortError(err)) return null;
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      appendSyncLog(`IPC command failed [${commandName}]: ${errorMessage}`);
      return null;
    } finally {
      activeCommands.current = Math.max(0, activeCommands.current - 1);
      if (activeCommands.current === 0) setIsLoading(false);
    }
  }, [appendSyncLog]);

  return {
    executeNativeCommand,
    isLoading,
    error,
    clearError: () => setError(null),
  };
}

const retryableCommands = new Set<NativeCommandName>([
  'get_ocr_capability',
  'list_saved_portals',
  'list_portal_runtimes',
  'get_corpus_index_status',
  'query_corpus_patterns',
  'get_agency_performance_profiles',
  'get_cross_agency_correlations',
  'list_legal_authorities',
  'list_request_documents',
  'preview_source_purge',
  'get_document_text_chunk',
  'list_document_provenance_verifications',
  'list_request_legal_findings',
]);

function defaultAttempts(command: NativeCommandName) {
  return retryableCommands.has(command) ? 2 : 1;
}

function abortableDelay(milliseconds: number, signal?: AbortSignal) {
  if (signal?.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'));
  return new Promise<void>((resolve, reject) => {
    const finish = () => {
      signal?.removeEventListener('abort', abort);
      resolve();
    };
    const timer = window.setTimeout(finish, milliseconds);
    const abort = () => {
      window.clearTimeout(timer);
      reject(new DOMException('Aborted', 'AbortError'));
    };
    signal?.addEventListener('abort', abort, { once: true });
  });
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === 'AbortError';
}
