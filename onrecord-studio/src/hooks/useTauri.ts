import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../store/useAppStore';

export function useTauri() {
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const appendSyncLog = useAppStore((s) => s.appendSyncLog);

  /**
   * Safe wrapper for invoking native asynchronous Rust backend commands.
   */
  const executeNativeCommand = useCallback(async <T>(
    commandName: string, 
    args: Record<string, any> = {}
  ): Promise<T | null> => {
    setIsLoading(true);
    setError(null);
    
    try {
      const result = await invoke<T>(commandName, args);
      return result;
    } catch (err: any) {
      const errorMessage = err?.message || String(err);
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
