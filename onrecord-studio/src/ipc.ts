import { invoke } from '@tauri-apps/api/core';
import type { NativeCommandMap } from './generated/nativeCommands';

export type { NativeCommandMap } from './generated/nativeCommands';

export type NativeCommandName = keyof NativeCommandMap;

export class NativeCommandError extends Error {
  readonly command: NativeCommandName;
  readonly details: unknown;

  constructor(command: NativeCommandName, details: unknown) {
    super(formatNativeError(details));
    this.name = 'NativeCommandError';
    this.command = command;
    this.details = details;
  }
}

export async function invokeCommand<Command extends NativeCommandName>(
  command: Command,
  args: NativeCommandMap[Command]['args'],
): Promise<NativeCommandMap[Command]['result']> {
  try {
    return await invoke<NativeCommandMap[Command]['result']>(command, args);
  } catch (error: unknown) {
    throw new NativeCommandError(command, error);
  }
}

export function formatNativeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (isRecord(error)) {
    const errorType = typeof error.errorType === 'string' ? error.errorType : null;
    const message = formatErrorMessage(error.message);
    if (errorType && message) return `${errorType}: ${message}`;
    if (message) return message;
  }
  try {
    const serialized = JSON.stringify(error);
    if (serialized && serialized !== '{}') return serialized;
  } catch {
    // Fall through to a stable message for non-serializable rejection values.
  }
  return 'The native command failed without a usable error message.';
}

function formatErrorMessage(message: unknown): string | null {
  if (typeof message === 'string') return message;
  if (message === null || message === undefined) return null;
  try {
    return JSON.stringify(message);
  } catch {
    return String(message);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
