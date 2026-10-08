import { describe, expect, it } from 'vitest';
import { NativeCommandError } from '../ipc';
import { displayError } from '../utils/errorMessage';

describe('displayError', () => {
  it('drops the backend code and the failing command name from what a reader sees, keeping both as detail', () => {
    const error = new NativeCommandError('get_profile', { errorType: 'DATABASE_UNAVAILABLE', message: 'The store could not answer get_profile: locked while reading audit_log.pdf.' });
    expect(displayError(error)).toEqual({
      message: 'The store could not answer this request: locked while reading audit_log.pdf.',
      code: 'DATABASE_UNAVAILABLE',
      detail: 'DATABASE_UNAVAILABLE: The store could not answer get_profile: locked while reading audit_log.pdf.',
    });
  });

  it('leaves an ordinary sentence with a colon intact', () => {
    expect(displayError(new Error('Network: unreachable host'))).toMatchObject({ message: 'Network: unreachable host', code: null });
  });
});
