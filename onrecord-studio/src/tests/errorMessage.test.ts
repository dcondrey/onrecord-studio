import { describe, expect, it } from 'vitest';
import { NativeCommandError } from '../ipc';
import { displayError } from '../utils/errorMessage';

describe('displayError', () => {
  it('drops the error class and the backend code from what a reader sees, keeping the code apart', () => {
    const error = new NativeCommandError('list_saved_portals', { errorType: 'DATABASE_UNAVAILABLE', message: 'The evidence store is locked.' });
    expect(displayError(error)).toEqual({ message: 'The evidence store is locked.', code: 'DATABASE_UNAVAILABLE' });
  });

  it('leaves an ordinary sentence with a colon intact', () => {
    expect(displayError(new Error('Network: unreachable host'))).toEqual({ message: 'Network: unreachable host', code: null });
  });
});
