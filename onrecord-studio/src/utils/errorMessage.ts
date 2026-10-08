import { formatNativeError } from '../ipc';

function isCodeCharacter(character: string): boolean {
  return (character >= 'A' && character <= 'Z') || (character >= '0' && character <= '9') || character === '_';
}

/**
 * A native failure as a reader sees it: the message without the leading `ERROR_CODE: ` the backend
 * prefixes. The code is kept apart so a `title` or the activity log can still carry it.
 */
export function displayError(error: unknown): { message: string; code: string | null } {
  const text = formatNativeError(error);
  const separator = text.indexOf(': ');
  if (separator < 2) return { message: text, code: null };
  const code = text.slice(0, separator);
  const message = text.slice(separator + 2).trim();
  if (!code.includes('_') || ![...code].every(isCodeCharacter) || !message) return { message: text, code: null };
  return { message, code };
}
