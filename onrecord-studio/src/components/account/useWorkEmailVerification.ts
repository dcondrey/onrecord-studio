import { useCallback, useEffect, useRef, useState } from 'react';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { invokeCommand } from '../../ipc';
import type {
  AffiliationLinkReport,
  AccountStatus,
  WorkEmailCodeReport,
  WorkEmailConfirmReport,
} from '../../types';

export const RESEND_COOLDOWN_SECONDS = 30;
export const CODE_LENGTH = 6;

export type Phase = 'idle' | 'sending' | 'sent' | 'confirming' | 'confirmed';

export interface Notice {
  tone: 'info' | 'warning' | 'error';
  text: string;
}

/** Digits only, at most six: a paste of "123 456" or "123-456" lands as 123456. */
export function sanitizeCode(raw: string): string {
  return raw.replace(/\D/g, '').slice(0, CODE_LENGTH);
}

function waitText(seconds: number): string {
  return seconds > 90 ? `about ${Math.ceil(seconds / 60)} minutes` : `${seconds} seconds`;
}

function codeNotice(report: WorkEmailCodeReport): Notice {
  switch (report.status) {
    case 'SENT': {
      const minutes = report.expiresInSeconds ? Math.round(report.expiresInSeconds / 60) : null;
      return {
        tone: 'info',
        text: `Code sent. ${minutes ? `It expires in ${minutes} minutes. ` : ''}Enter it below.`,
      };
    }
    case 'INVALID_EMAIL':
      return {
        tone: 'error',
        text: 'The server does not accept that address: use plain letters, digits and standard punctuation, with no spaces or non-ASCII characters.',
      };
    case 'RATE_LIMITED':
      return {
        tone: 'warning',
        text: `Too many codes were requested from this account. Try again in ${waitText(report.retryAfterSeconds ?? 3600)}.`,
      };
    case 'NOT_SIGNED_IN':
      return { tone: 'warning', text: 'Sign in from the Account menu first; a code can only be sent to a signed-in account.' };
    case 'SEND_FAILED':
      return { tone: 'error', text: 'The message could not be sent. Try again later; any earlier code no longer works.' };
  }
}

function confirmNotice(report: WorkEmailConfirmReport): Notice {
  switch (report.status) {
    case 'VERIFIED':
      return report.isFreeMail
        ? {
            tone: 'warning',
            text: `Confirmed a mailbox at ${report.domain}. That is a consumer mail provider: recorded, not used as proof of any organization.`,
          }
        : {
            tone: 'info',
            text: `Confirmed control of a mailbox at ${report.domain}. This does not show employment. Link it to an organization below if you choose.`,
          };
    case 'INVALID_INPUT':
      return { tone: 'error', text: 'Enter the six-digit code and the address it was sent to.' };
    case 'NOT_SIGNED_IN':
      return { tone: 'warning', text: 'Sign in from the Account menu first.' };
    case 'NO_PENDING_REQUEST':
      return { tone: 'warning', text: 'No code was requested for this address in this session. Send a code first.' };
    case 'CODE_INVALID':
      return {
        tone: 'error',
        text:
          report.attemptsRemaining === 0
            ? 'That code is wrong and no attempts are left. Send a new code.'
            : report.attemptsRemaining !== null
              ? `That code is wrong. ${report.attemptsRemaining} ${report.attemptsRemaining === 1 ? 'attempt' : 'attempts'} left.`
              : 'That code is wrong.',
      };
    case 'EXPIRED_OR_EXHAUSTED':
      return { tone: 'warning', text: 'The code expired, was already used, or ran out of attempts. Send a new code.' };
    case 'RATE_LIMITED':
      return { tone: 'warning', text: `Too many attempts. Try again in ${waitText(report.retryAfterSeconds ?? 3600)}.` };
  }
}

/**
 * State for one affiliation's work-email step. Every async result is checked against a request
 * token, and editing the address invalidates whatever is in flight: a code sent for one address
 * must never be reported as confirming another.
 */
export function useWorkEmailVerification(onProfileChanged: () => void) {
  const actionGate = useLatestRequestGate();
  const statusGate = useLatestRequestGate();
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [email, setEmailState] = useState('');
  const [code, setCodeState] = useState('');
  const [phase, setPhase] = useState<Phase>('idle');
  const [notice, setNotice] = useState<Notice | null>(null);
  const [confirmed, setConfirmed] = useState<WorkEmailConfirmReport | null>(null);
  const [cooldownEndsAt, setCooldownEndsAt] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const emailRef = useRef(email);

  useEffect(() => {
    const token = statusGate.begin();
    invokeCommand('get_account_status', {})
      .then((status) => {
        if (statusGate.isCurrent(token)) setAccount(status);
      })
      // Unknown is not "signed out": the server still refuses an unauthenticated request.
      .catch(() => undefined);
  }, [statusGate]);

  useEffect(() => {
    if (cooldownEndsAt <= Date.now()) return undefined;
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [cooldownEndsAt]);

  const cooldownSeconds = Math.max(0, Math.ceil((cooldownEndsAt - now) / 1000));

  const setEmail = useCallback(
    (value: string) => {
      if (value === emailRef.current) return;
      emailRef.current = value;
      actionGate.invalidate();
      setEmailState(value);
      setCodeState('');
      setPhase('idle');
      setNotice(null);
      setConfirmed(null);
    },
    [actionGate],
  );

  const setCode = useCallback((value: string) => setCodeState(sanitizeCode(value)), []);

  const fail = (reason: unknown) => setNotice({ tone: 'error', text: String(reason) });

  const sendCode = useCallback(() => {
    const token = actionGate.begin();
    const requested = emailRef.current;
    setPhase('sending');
    setNotice(null);
    setConfirmed(null);
    invokeCommand('request_work_email_code', { email: requested })
      .then((report) => {
        if (!actionGate.isCurrent(token) || emailRef.current !== requested) return;
        setNotice(codeNotice(report));
        setPhase(report.status === 'SENT' ? 'sent' : 'idle');
        setCodeState('');
        const wait =
          report.status === 'SENT'
            ? RESEND_COOLDOWN_SECONDS
            : report.status === 'RATE_LIMITED'
              ? (report.retryAfterSeconds ?? 3600)
              : 0;
        setNow(Date.now());
        setCooldownEndsAt(wait > 0 ? Date.now() + wait * 1000 : 0);
      })
      .catch((reason) => {
        if (!actionGate.isCurrent(token) || emailRef.current !== requested) return;
        setPhase('idle');
        fail(reason);
      });
  }, [actionGate]);

  const confirmCode = useCallback(() => {
    const token = actionGate.begin();
    const requested = emailRef.current;
    setPhase('confirming');
    setNotice(null);
    invokeCommand('confirm_work_email_code', { email: requested, code })
      .then((report) => {
        if (!actionGate.isCurrent(token) || emailRef.current !== requested) return;
        setNotice(confirmNotice(report));
        if (report.status === 'VERIFIED') {
          setConfirmed(report);
          setPhase('confirmed');
          setCodeState('');
          onProfileChanged();
        } else {
          setPhase(report.status === 'CODE_INVALID' && report.attemptsRemaining !== 0 ? 'sent' : 'idle');
          if (report.status === 'RATE_LIMITED') {
            setNow(Date.now());
            setCooldownEndsAt(Date.now() + (report.retryAfterSeconds ?? 3600) * 1000);
          }
        }
      })
      .catch((reason) => {
        if (!actionGate.isCurrent(token) || emailRef.current !== requested) return;
        setPhase('sent');
        fail(reason);
      });
  }, [actionGate, code, onProfileChanged]);

  /** The user's explicit link. The server-free profile command decides whether the names match. */
  const link = useCallback(
    (affiliationId: string, domain: string): Promise<AffiliationLinkReport | null> => {
      const token = actionGate.begin();
      return invokeCommand('link_affiliation_verified_domain', { input: { affiliationId, domain } })
        .then((report) => {
          if (!actionGate.isCurrent(token)) return null;
          setNotice({ tone: report.status === 'LINKED' ? 'info' : 'warning', text: report.message });
          if (report.status === 'LINKED') onProfileChanged();
          return report;
        })
        .catch((reason) => {
          if (actionGate.isCurrent(token)) fail(reason);
          return null;
        });
    },
    [actionGate, onProfileChanged],
  );

  return {
    account,
    email,
    setEmail,
    code,
    setCode,
    phase,
    notice,
    confirmed,
    cooldownSeconds,
    sendCode,
    confirmCode,
    link,
  };
}
