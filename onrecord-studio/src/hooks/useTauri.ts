import { useState, useCallback } from 'react';
import { invokeCommand, type NativeCommandMap, type NativeCommandName } from '../ipc';
import { useAppStore } from '../store/useAppStore';

export function useTauri() {
  // TODO(ipc): Add command-specific retry policy and cancellation support here
  // instead of requiring each screen to recreate bridge error handling.
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const appendSyncLog = useAppStore((s) => s.appendSyncLog);

  /**
   * Safe wrapper for invoking native asynchronous Rust backend commands.
   */
  const executeNativeCommand = useCallback(async <Command extends NativeCommandName>(
    commandName: Command,
    args: NativeCommandMap[Command]['args'],
  ): Promise<NativeCommandMap[Command]['result'] | null> => {
    setIsLoading(true);
    setError(null);
    
    try {
      const result = await invokeCommand(commandName, args);
      return result;
    } catch (err: unknown) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      appendSyncLog(`❌ IPC BRIDGE BREAKDOWN [${commandName}]: ${errorMessage}`);
      return null;
    } finally {
      setIsLoading(false);
    }
  }, [appendSyncLog]);

  return {
    executeNativeCommand,
    isLoading,
    error,
    clearError: () => setError(null)
  };
}
