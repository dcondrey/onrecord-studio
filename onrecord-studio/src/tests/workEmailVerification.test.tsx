// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import WorkEmailVerification, {
  VERIFIED_BADGE_TEXT,
} from '../components/account/WorkEmailVerification';
import { sanitizeCode } from '../components/account/useWorkEmailVerification';
import * as ipc from '../ipc';
import type {
  AccountStatus,
  Affiliation,
  AffiliationLinkReport,
  VerifiedDomain,
  WorkEmailCodeReport,
  WorkEmailConfirmReport,
} from '../types';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const signedIn: AccountStatus = { available: true, signedIn: true, email: null, provider: null };
const affiliation: Affiliation = {
  id: 'a1',
  name: 'Acme',
  aliases: [],
  basis: 'SELF_DECLARED',
  createdAt: 't',
  verifiedDomain: null,
};
const sent: WorkEmailCodeReport = { status: 'SENT', expiresInSeconds: 600, retryAfterSeconds: null };
const confirmed = (over: Partial<WorkEmailConfirmReport> = {}): WorkEmailConfirmReport => ({
  status: 'VERIFIED',
  domain: 'acme.com',
  isFreeMail: false,
  attemptsRemaining: null,
  verifiedAt: 't',
  retryAfterSeconds: null,
  ...over,
});

type Handler = (command: string, args: unknown) => unknown;

function mockIpc(handler: Handler, account: AccountStatus = signedIn) {
  return vi.spyOn(ipc, 'invokeCommand').mockImplementation(((command: string, args: unknown) => {
    if (command === 'get_account_status') return Promise.resolve(account);
    return Promise.resolve(handler(command, args));
  }) as unknown as typeof ipc.invokeCommand);
}

function mount(
  props: Partial<{ affiliation: Affiliation | null; verifiedDomains: VerifiedDomain[] }> = {},
) {
  const changed = vi.fn();
  render(
    <WorkEmailVerification
      affiliation={'affiliation' in props ? props.affiliation! : affiliation}
      verifiedDomains={props.verifiedDomains ?? []}
      onProfileChanged={changed}
    />,
  );
  return changed;
}

const emailField = () => screen.getByLabelText('Work email address') as HTMLInputElement;
const button = (name: string | RegExp) =>
  screen.getByRole('button', { name }) as HTMLButtonElement;
const codeField = () => screen.getByLabelText('Six-digit code') as HTMLInputElement;

async function sendTo(address: string) {
  fireEvent.change(emailField(), { target: { value: address } });
  await waitFor(() => expect(button('Send code').disabled).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: 'Send code' }));
}

describe('work email verification step', () => {
  test('a pasted code keeps only six digits and autofill is enabled', async () => {
    expect(sanitizeCode('12 34-56789')).toBe('123456');
    expect(sanitizeCode('12345')).toBe('12345');
    mockIpc(() => sent);
    mount();
    await sendTo('me@acme.com');
    await waitFor(() => expect(codeField()).not.toBeNull());
    expect(codeField().getAttribute('autocomplete')).toBe('one-time-code');
    expect(codeField().getAttribute('inputmode')).toBe('numeric');
    fireEvent.change(codeField(), { target: { value: '12 345' } });
    expect(button('Confirm code').disabled).toBe(true);
    fireEvent.change(codeField(), { target: { value: '123 456 789' } });
    expect(codeField().value).toBe('123456');
    expect(button('Confirm code').disabled).toBe(false);
  });

  test('signed out: explains sign-in and cannot send', async () => {
    const spy = mockIpc(() => sent, { available: true, signedIn: false, email: null, provider: null });
    mount();
    await screen.findByText(/needs a signed-in account/);
    fireEvent.change(emailField(), { target: { value: 'me@acme.com' } });
    expect(button('Send code').disabled).toBe(true);
    expect(spy.mock.calls.some(([command]) => command === 'request_work_email_code')).toBe(false);
  });

  test('rate limited: states the wait and disables resending', async () => {
    mockIpc(() => ({ status: 'RATE_LIMITED', expiresInSeconds: null, retryAfterSeconds: 3600 }));
    mount();
    await sendTo('me@acme.com');
    await screen.findByText(/Try again in about 60 minutes/);
    expect(button(/Send again in/).disabled).toBe(true);
  });

  test('a successful send starts a resend cooldown', async () => {
    mockIpc(() => sent);
    mount();
    await sendTo('me@acme.com');
    await screen.findByText(/Code sent/);
    expect(button(/Send again in \d+s/).disabled).toBe(true);
  });

  test('a wrong code says how many attempts are left', async () => {
    mockIpc((command) =>
      command === 'request_work_email_code'
        ? sent
        : confirmed({ status: 'CODE_INVALID', domain: null, verifiedAt: null, attemptsRemaining: 2 }),
    );
    mount();
    await sendTo('me@acme.com');
    fireEvent.change(await screen.findByLabelText('Six-digit code'), { target: { value: '111111' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm code' }));
    await screen.findByText('That code is wrong. 2 attempts left.');
  });

  test('an expired code asks for a new one', async () => {
    mockIpc((command) =>
      command === 'request_work_email_code'
        ? sent
        : confirmed({ status: 'EXPIRED_OR_EXHAUSTED', domain: null, verifiedAt: null }),
    );
    mount();
    await sendTo('me@acme.com');
    fireEvent.change(await screen.findByLabelText('Six-digit code'), { target: { value: '111111' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm code' }));
    await screen.findByText(/expired, was already used, or ran out of attempts/);
  });

  test('a free-mail confirmation is recorded, not used as proof, and offers no link', async () => {
    mockIpc((command) =>
      command === 'request_work_email_code' ? sent : confirmed({ domain: 'gmail.com', isFreeMail: true }),
    );
    const changed = mount();
    await sendTo('me@gmail.com');
    fireEvent.change(await screen.findByLabelText('Six-digit code'), { target: { value: '123456' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm code' }));
    await screen.findByText(/recorded, not used as proof of any organization/);
    expect(changed).toHaveBeenCalled();
    expect(screen.queryByText(VERIFIED_BADGE_TEXT, { exact: false })).toBeNull();
  });

  test('a free-mail domain in the profile has no link control', () => {
    mockIpc(() => sent);
    mount({ verifiedDomains: [{ domain: 'gmail.com', verifiedAt: 't', isFreeMail: true }] });
    expect(screen.queryByRole('button', { name: 'Link to this organization' })).toBeNull();
    expect(screen.getByText(/recorded, not used as proof/)).not.toBeNull();
  });

  test('linking needs the explicit choice and a non-match stays self-declared with the reason', async () => {
    const report: AffiliationLinkReport = {
      status: 'NOT_MATCHING',
      message: 'Neither the name nor an alias equals what acme.com names. It stays self-declared.',
      profile: { status: 'SET', profile: null },
    };
    const spy = mockIpc(() => report);
    const changed = mount({ verifiedDomains: [{ domain: 'acme.com', verifiedAt: 't', isFreeMail: false }] });
    const link = screen.getByRole('button', { name: 'Link to this organization' });
    expect((link as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(link);
    await screen.findByText(/It stays self-declared/);
    expect(spy).toHaveBeenCalledWith('link_affiliation_verified_domain', {
      input: { affiliationId: 'a1', domain: 'acme.com' },
    });
    expect(changed).not.toHaveBeenCalled();
    expect(screen.queryByText(VERIFIED_BADGE_TEXT, { exact: false })).toBeNull();
  });

  test('the verified badge states what it does and does not prove', () => {
    mockIpc(() => sent);
    mount({ affiliation: { ...affiliation, basis: 'EMAIL_DOMAIN_VERIFIED', verifiedDomain: 'acme.com' } });
    expect(
      screen.getByText('Email domain verified (control of a mailbox, not employment): acme.com'),
    ).not.toBeNull();
  });

  test('an unsaved organization cannot start verification', () => {
    mockIpc(() => sent);
    mount({ affiliation: null });
    expect(screen.getByText(/Save the profile before verifying/)).not.toBeNull();
    expect(screen.queryByLabelText('Work email address')).toBeNull();
  });

  test('a send that resolves after the address was edited is discarded', async () => {
    const pending: Array<(report: WorkEmailCodeReport) => void> = [];
    mockIpc((command) =>
      command === 'request_work_email_code'
        ? new Promise<WorkEmailCodeReport>((resolve) => pending.push(resolve))
        : undefined,
    );
    mount();
    await sendTo('first@acme.com');
    await waitFor(() => expect(pending).toHaveLength(1));
    fireEvent.change(emailField(), { target: { value: 'second@other.org' } });
    await act(async () => {
      pending[0](sent);
    });
    expect(screen.queryByText(/Code sent/)).toBeNull();
    expect(screen.queryByLabelText('Six-digit code')).toBeNull();
  });

  test('a confirmation that resolves after the address was edited is not shown or recorded', async () => {
    const pending: Array<(report: WorkEmailConfirmReport) => void> = [];
    const spy = mockIpc((command) =>
      command === 'request_work_email_code'
        ? sent
        : new Promise<WorkEmailConfirmReport>((resolve) => pending.push(resolve)),
    );
    const changed = mount();
    await sendTo('first@acme.com');
    fireEvent.change(await screen.findByLabelText('Six-digit code'), { target: { value: '123456' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm code' }));
    await waitFor(() => expect(pending).toHaveLength(1));
    fireEvent.change(emailField(), { target: { value: 'second@other.org' } });
    await act(async () => {
      pending[0](confirmed());
    });
    expect(screen.queryByText(/Confirmed control of a mailbox/)).toBeNull();
    expect(changed).not.toHaveBeenCalled();
    expect(spy.mock.calls.filter(([command]) => command === 'link_affiliation_verified_domain')).toHaveLength(0);
  });
});
