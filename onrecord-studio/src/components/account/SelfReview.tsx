import { useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import type { Finding, FindingKind } from '../../types';
import { useDialogFocus } from '../licensing/useDialogFocus';
import { touchesOwnOrganization, useAffiliationDisclosure } from './useProfile';

export interface ReviewGroup {
  title: string;
  kinds: FindingKind[];
}

export const REVIEW_GROUPS: ReviewGroup[] = [
  {
    title: 'Contradictions in the record',
    kinds: ['CONTRADICTORY_STATEMENT', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'METADATA_CONTRADICTS_CONTENT'],
  },
  {
    title: 'Missing records',
    kinds: ['ABSENCE_OF_RECORDS', 'HELD_BACK_RESPONSE', 'EMAIL_THREAD_GAP'],
  },
  {
    title: 'Late responses',
    kinds: ['JUST_IN_TIME_COMPLIANCE', 'EXTENSION_ABUSE', 'EVASION_LATE_ARRIVING_RECORDS', 'POLICY_VIOLATION_DETECTED'],
  },
  {
    title: 'PII-masking and redaction gaps',
    kinds: ['EVASION_REDACTION_AT_LIMIT', 'REDACTION_CANDIDATE_INFERRED'],
  },
];

export function groupOwnFindings(findings: Finding[]) {
  return REVIEW_GROUPS.map((group) => ({
    ...group,
    findings: findings.filter((finding) => group.kinds.includes(finding.kind)),
  }));
}

export default function SelfReview() {
  const open = useProfileLensStore((state) => state.selfReviewOpen);
  const setOpen = useProfileLensStore((state) => state.setSelfReviewOpen);
  if (!open) return null;
  return <SelfReviewPanel onClose={() => setOpen(false)} />;
}

export function SelfReviewPanel({ onClose }: { onClose: () => void }) {
  const { disclosure } = useAffiliationDisclosure();
  const dialogFocus = useDialogFocus(onClose);
  const gate = useLatestRequestGate();
  const [findings, setFindings] = useState<Finding[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const token = gate.begin();
    invokeCommand('list_findings', { filter: null, limit: 500 })
      .then((rows) => {
        if (gate.isCurrent(token)) setFindings(rows);
      })
      .catch((reason) => {
        if (gate.isCurrent(token)) setError(formatNativeError(reason));
      });
  }, [gate]);

  const own = useMemo(
    () => (findings ?? []).filter((finding) => touchesOwnOrganization(finding, disclosure)),
    [findings, disclosure],
  );
  const hasConfirmedSubject =
    disclosure !== null && (disclosure.ownPortalKeys.length > 0 ||
      disclosure.ownAgencies.length > 0 ||
      disclosure.ownEntityIds.length > 0);

  return (
    <div role="dialog" aria-modal="true" aria-label="Records self-review" className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4" onKeyDown={dialogFocus.onKeyDown}>
      <div ref={dialogFocus.ref} tabIndex={-1} className="app-panel max-h-[90vh] w-full max-w-2xl overflow-y-auto p-4 text-label text-fg-secondary outline-none">
        <div className="flex items-center justify-between">
          <h2 className="text-label font-semibold text-fg">What the public record shows about your organization</h2>
          <button type="button" onClick={onClose} className="focus-ring rounded border border-control px-2 py-1">
            Close
          </button>
        </div>
        <p className="mt-2 text-label leading-relaxed text-muted">
          A records-hygiene view: gaps and inconsistencies a requester could find, so you can
          correct the record and respond completely. Your profile did not change any of these
          findings, their confidence or their severity.
        </p>
        {error && <div role="alert" className="mt-2 text-danger">{error}</div>}
        {disclosure && disclosure.status === 'UNSET' && (
          <p role="status" className="mt-3">No profile is set. Set one and confirm a match to see this view.</p>
        )}
        {disclosure && disclosure.status === 'DECLARED' && !hasConfirmedSubject && (
          <p role="status" className="mt-3">
            No match is confirmed yet, so no findings can be attributed to your organization. This is
            not a statement that the record is clean.
          </p>
        )}
        {hasConfirmedSubject && findings && (
          <div className="mt-3 flex flex-col gap-3">
            {groupOwnFindings(own).map((group) => (
              <section key={group.title} aria-label={group.title}>
                <h3 className="font-semibold text-fg">
                  {group.title} ({group.findings.length})
                </h3>
                {group.findings.length === 0 ? (
                  <p className="text-muted">None among the findings currently listed.</p>
                ) : (
                  <ul className="mt-1 flex flex-col gap-1">
                    {group.findings.map((finding) => (
                      <li key={finding.id} className="rounded border border-line p-2">
                        <div className="font-semibold text-fg">
                          {finding.headline}
                          {finding.kind === 'REDACTION_CANDIDATE_INFERRED' && ' (hypothesis, not established)'}
                        </div>
                        <div className="text-muted">{finding.explanation}</div>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
