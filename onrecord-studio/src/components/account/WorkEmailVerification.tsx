import { useId, useState } from 'react';
import { type Affiliation, type VerifiedDomain } from '../../types';
import {
  CODE_LENGTH,
  useWorkEmailVerification,
} from './useWorkEmailVerification';
import Button from '../ui/Button';
import { Input } from '../ui/Field';

export const VERIFIED_BADGE_TEXT = 'Email domain verified (control of a mailbox, not employment)';

const noticeTone = {
  info: 'text-fg-secondary',
  warning: 'text-warning',
  error: 'text-danger',
} as const;

/**
 * One affiliation's "Verify with a work email" step. A confirmed code proves control of a mailbox
 * at a domain. It is never employment, and a consumer mail domain is recorded but never proof.
 */
export default function WorkEmailVerification({
  affiliation,
  verifiedDomains,
  onProfileChanged,
}: {
  affiliation: Affiliation | null;
  verifiedDomains: VerifiedDomain[];
  onProfileChanged: () => void;
}) {
  const verification = useWorkEmailVerification(onProfileChanged);
  const [linkConsent, setLinkConsent] = useState<string | null>(null);
  const emailId = useId();
  const codeId = useId();
  const { account, phase, cooldownSeconds } = verification;
  const busy = phase === 'sending' || phase === 'confirming';
  const accountUnavailable = account !== null && !account.available;
  const signedOut = account !== null && account.available && !account.signedIn;
  const linkable = verifiedDomains.filter(
    (item) => !item.isFreeMail && item.domain !== affiliation?.verifiedDomain,
  );
  const freeMail = verifiedDomains.filter((item) => item.isFreeMail);

  if (!affiliation) {
    return <p className="mt-3 text-caption text-muted">Save the profile before verifying this organization.</p>;
  }

  return (
    <section aria-label={`Verify ${affiliation.name} with a work email`} className="mt-4 border-t border-line pt-4">
      <h4 className="text-control text-fg">Verify with a work email</h4>
      {affiliation.basis === 'EMAIL_DOMAIN_VERIFIED' ? (
        <p role="status" className="mt-1 text-caption text-success">
          {VERIFIED_BADGE_TEXT}
          {affiliation.verifiedDomain ? `: ${affiliation.verifiedDomain}` : ''}
        </p>
      ) : (
        <p className="mt-1 text-caption text-muted">Self-declared, not verified.</p>
      )}
      <p className="mt-1 text-caption text-muted">
        A code sent to a mailbox proves you control that mailbox, not that you work there. Only the
        domain is kept on this device, never the address or the code.
      </p>
      {accountUnavailable && (
        <p role="status" className="mt-2 text-caption text-warning">
          Accounts are not part of this build, so a work email cannot be verified here.
        </p>
      )}
      {signedOut && (
        <p role="status" className="mt-2 text-caption text-warning">
          Sending a code needs a signed-in account. Sign in from the Account menu in the top bar,
          then return here.
        </p>
      )}
      <label htmlFor={emailId} className="mt-3 block text-control text-fg-secondary">Work email address</label>
      <div className="mt-1.5 flex items-center gap-2">
        <Input
          id={emailId}
          type="email"
          autoComplete="email"
          maxLength={254}
          value={verification.email}
          onChange={(event) => verification.setEmail(event.target.value)}
          className="min-w-0 flex-1"
        />
        <Button
          disabled={busy || accountUnavailable || signedOut || verification.email.trim() === '' || cooldownSeconds > 0}
          onClick={verification.sendCode}
          className="tabular-nums"
        >
          {cooldownSeconds > 0
            ? `Send again in ${cooldownSeconds > 90 ? `${Math.ceil(cooldownSeconds / 60)} min` : `${cooldownSeconds}s`}`
            : phase === 'sent' || phase === 'confirming'
              ? 'Send a new code'
              : 'Send code'}
        </Button>
      </div>
      {(phase === 'sent' || phase === 'confirming') && (
        <>
          <label htmlFor={codeId} className="mt-3 block text-control text-fg-secondary">Six-digit code</label>
          <div className="mt-1.5 flex items-center gap-2">
            <Input
              id={codeId}
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9]*"
              maxLength={CODE_LENGTH}
              value={verification.code}
              onChange={(event) => verification.setCode(event.target.value)}
              className="w-32 tabular-nums tracking-widest"
            />
            <Button disabled={busy || verification.code.length !== CODE_LENGTH} onClick={verification.confirmCode}>
              Confirm code
            </Button>
          </div>
        </>
      )}
      {verification.notice && (
        <p
          role={verification.notice.tone === 'error' ? 'alert' : 'status'}
          className={`mt-2 text-caption ${noticeTone[verification.notice.tone]}`}
        >
          {verification.notice.text}
        </p>
      )}
      {affiliation.basis !== 'EMAIL_DOMAIN_VERIFIED' && linkable.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1" aria-label="Verified domains">
          {linkable.map((item) => (
            <li key={item.domain} className="rounded-lg border border-line p-3">
              <div>
                Mailbox confirmed at <strong>{item.domain}</strong>
              </div>
              <label className="mt-1 flex items-start gap-1 text-label text-fg-secondary">
                <input
                  type="checkbox"
                  checked={linkConsent === item.domain}
                  onChange={(event) => setLinkConsent(event.target.checked ? item.domain : null)}
                />
                <span>
                  Link {affiliation.name} to {item.domain}. This records control of a mailbox at that
                  domain, not employment, and applies only if the domain&apos;s name equals this
                  organization&apos;s name or an alias.
                </span>
              </label>
              <Button size="sm" disabled={linkConsent !== item.domain} onClick={() => void verification.link(affiliation.id, item.domain)} className="mt-2">
                Link to this organization
              </Button>
            </li>
          ))}
        </ul>
      )}
      {freeMail.length > 0 && (
        <p className="mt-2 text-caption text-muted">
          Consumer mail domains ({freeMail.map((item) => item.domain).join(', ')}) are recorded, not
          used as proof.
        </p>
      )}
    </section>
  );
}
