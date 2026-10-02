import { useId, useState } from 'react';
import { type Affiliation, type VerifiedDomain } from '../../types';
import {
  CODE_LENGTH,
  useWorkEmailVerification,
} from './useWorkEmailVerification';

export const VERIFIED_BADGE_TEXT = 'Email domain verified (control of a mailbox, not employment)';

const noticeTone = {
  info: 'text-slate-300',
  warning: 'text-amber-300',
  error: 'text-rose-300',
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
    return <p className="mt-2 text-label text-slate-400">Save the profile before verifying this organization.</p>;
  }

  return (
    <section aria-label={`Verify ${affiliation.name} with a work email`} className="mt-2 border-t border-slate-800 pt-2">
      <h4 className="font-semibold text-slate-200">Verify with a work email</h4>
      {affiliation.basis === 'EMAIL_DOMAIN_VERIFIED' ? (
        <p role="status" className="mt-1 rounded border border-emerald-500/40 px-2 py-1 text-emerald-200">
          {VERIFIED_BADGE_TEXT}
          {affiliation.verifiedDomain ? `: ${affiliation.verifiedDomain}` : ''}
        </p>
      ) : (
        <p className="mt-1 text-label text-slate-400">Self-declared, not verified.</p>
      )}
      <p className="mt-1 text-label leading-relaxed text-slate-400">
        A code sent to a mailbox proves you control that mailbox, not that you work there. Only the
        domain is kept on this device, never the address or the code.
      </p>
      {accountUnavailable && (
        <p role="status" className="mt-1 text-amber-300">
          Accounts are not part of this build, so a work email cannot be verified here.
        </p>
      )}
      {signedOut && (
        <p role="status" className="mt-1 text-amber-300">
          Sending a code needs a signed-in account. Sign in from the Account menu in the top bar,
          then return here.
        </p>
      )}
      <div className="mt-1 flex flex-wrap items-end gap-2">
        <label htmlFor={emailId} className="block">
          Work email address
          <input
            id={emailId}
            type="email"
            autoComplete="email"
            maxLength={254}
            value={verification.email}
            onChange={(event) => verification.setEmail(event.target.value)}
            className="mt-1 block w-64 rounded border border-slate-700 bg-slate-950 px-2 py-1"
          />
        </label>
        <button
          type="button"
          disabled={busy || accountUnavailable || signedOut || verification.email.trim() === '' || cooldownSeconds > 0}
          onClick={verification.sendCode}
          className="focus-ring rounded border border-cyan-400/40 px-2 py-1 text-cyan-200 disabled:opacity-50"
        >
          {cooldownSeconds > 0
            ? `Send again in ${cooldownSeconds > 90 ? `${Math.ceil(cooldownSeconds / 60)} min` : `${cooldownSeconds}s`}`
            : phase === 'sent' || phase === 'confirming'
              ? 'Send a new code'
              : 'Send code'}
        </button>
      </div>
      {(phase === 'sent' || phase === 'confirming') && (
        <div className="mt-2 flex flex-wrap items-end gap-2">
          <label htmlFor={codeId} className="block">
            Six-digit code
            <input
              id={codeId}
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9]*"
              maxLength={CODE_LENGTH}
              value={verification.code}
              onChange={(event) => verification.setCode(event.target.value)}
              className="mt-1 block w-28 rounded border border-slate-700 bg-slate-950 px-2 py-1 tracking-widest"
            />
          </label>
          <button
            type="button"
            disabled={busy || verification.code.length !== CODE_LENGTH}
            onClick={verification.confirmCode}
            className="focus-ring rounded border border-cyan-400/40 px-2 py-1 text-cyan-200 disabled:opacity-50"
          >
            Confirm code
          </button>
        </div>
      )}
      {verification.notice && (
        <p
          role={verification.notice.tone === 'error' ? 'alert' : 'status'}
          className={`mt-1 ${noticeTone[verification.notice.tone]}`}
        >
          {verification.notice.text}
        </p>
      )}
      {affiliation.basis !== 'EMAIL_DOMAIN_VERIFIED' && linkable.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1" aria-label="Verified domains">
          {linkable.map((item) => (
            <li key={item.domain} className="rounded border border-slate-800 p-2">
              <div>
                Mailbox confirmed at <strong>{item.domain}</strong>
              </div>
              <label className="mt-1 flex items-start gap-1 text-label text-slate-300">
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
              <button
                type="button"
                disabled={linkConsent !== item.domain}
                onClick={() => void verification.link(affiliation.id, item.domain)}
                className="focus-ring mt-1 rounded border border-slate-700 px-2 py-0.5 disabled:opacity-50"
              >
                Link to this organization
              </button>
            </li>
          ))}
        </ul>
      )}
      {freeMail.length > 0 && (
        <p className="mt-1 text-label text-slate-400">
          Consumer mail domains ({freeMail.map((item) => item.domain).join(', ')}) are recorded, not
          used as proof.
        </p>
      )}
    </section>
  );
}
