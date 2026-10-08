import { formatNativeError } from '../ipc';

function isCodeCharacter(character: string): boolean {
  return (character >= 'A' && character <= 'Z') || (character >= '0' && character <= '9') || character === '_';
}

/**
 * A native failure as a reader sees it: the message without the leading `ERROR_CODE: ` the backend
 * prefixes, and with the name of the command that failed replaced by "this request". The code and
 * the raw text are kept apart so a `title`, "Technical details" or the activity log can carry them.
 */
export function displayError(error: unknown): { message: string; code: string | null; detail: string } {
  const detail = formatNativeError(error);
  let code: string | null = null;
  let message = detail;
  const separator = detail.indexOf(': ');
  if (separator >= 2) {
    const candidate = detail.slice(0, separator);
    const rest = detail.slice(separator + 2).trim();
    if (candidate.includes('_') && [...candidate].every(isCodeCharacter) && rest) {
      code = candidate;
      message = rest;
    }
  }
  // A NativeCommandError names the command it wrapped; read structurally so a stubbed bridge works too.
  const command = error instanceof Error ? (error as Error & { command?: unknown }).command : undefined;
  if (typeof command === 'string' && command) message = message.split(command).join('this request');
  return { message, code, detail };
}
