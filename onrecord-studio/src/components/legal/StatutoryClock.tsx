import type { DeterminationBasis, StatuteCitation, StatutoryClockRule, StatutoryPeriod } from '../../types';

/** A source's stored jurisdiction: a rule, `NONE` (no modeled statute applies), or unset. */
export type StoredJurisdiction = 'CA_CPRA' | 'US_FOIA' | 'NONE' | null;

export function periodText(period: StatutoryPeriod) {
  return `${period.days} ${period.kind === 'WORKING_DAYS' ? 'working' : 'calendar'} days`;
}

function Cite({ citation }: { citation: StatuteCitation }) {
  return (
    <a href={citation.url} target="_blank" rel="noreferrer" className="text-highlight underline">
      {citation.section}
    </a>
  );
}

/** The rule a clock was measured under, with the citation behind each part. */
export function ClockRuleSummary({ rule }: { rule: StatutoryClockRule }) {
  return (
    <div className="mt-1 text-caption leading-relaxed text-muted">
      <div className="font-semibold text-fg-secondary">{rule.title}</div>
      <p>
        Determination within {periodText(rule.initial)} of {rule.start.event} (<Cite citation={rule.initialCitation} />
        {rule.start.latestAfterReceipt && (
          <>
            ; counted from {periodText(rule.start.latestAfterReceipt)} after first receipt, the latest start{' '}
            <Cite citation={rule.start.citation} /> allows
          </>
        )}
        ). One written extension of at most {periodText(rule.extension.maximum)} (<Cite citation={rule.extension.citation} />).
        Closed days: <Cite citation={rule.calendarCitation} />.
      </p>
      {rule.unrecordedClockStops && (
        <p>
          The clock may also pause under <Cite citation={rule.unrecordedClockStops} />, which no record here shows.
        </p>
      )}
    </div>
  );
}

const BASIS_TEXT: Record<DeterminationBasis, string> = {
  SOURCED_DETERMINATION_EVENT: 'Determination: recorded event, with its source.',
  EARLIEST_RECORDED_ACTIVITY_PROXY:
    'Determination: proxy. No determination is recorded; the earliest recorded activity stands in for it.',
  OBSERVATION_WITHOUT_RECORDED_ACTIVITY:
    'Determination: none recorded. The period is measured to the evaluation date and is still running.',
};

export function DeterminationBasisNote({ basis }: { basis: DeterminationBasis | null }) {
  return (
    <p className="mt-1 text-caption leading-relaxed text-muted">
      {basis ? BASIS_TEXT[basis] : 'Determination basis: not recorded on this finding.'}
    </p>
  );
}

interface StatutoryClockSectionProps {
  rules: StatutoryClockRule[] | null;
  jurisdiction: StoredJurisdiction | undefined;
  loadError: string | null;
  isSaving: boolean;
  onSelect: (jurisdiction: 'CA_CPRA' | 'US_FOIA' | 'NONE') => void;
}

/** Which statute's clock this source's requests are measured under, and the means to change it. */
export function StatutoryClockSection({
  rules,
  jurisdiction,
  loadError,
  isSaving,
  onSelect,
}: StatutoryClockSectionProps) {
  const rule = rules?.find((candidate) => candidate.jurisdiction === jurisdiction) ?? null;
  return (
    <div className="border-b border-line px-3 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-caption font-bold uppercase tracking-[0.16em] text-muted">Statutory clock</div>
          {loadError ? (
            <p className="mt-1 text-caption leading-relaxed text-danger">
              The source&apos;s jurisdiction could not be read, so this panel cannot say which clock applies.
            </p>
          ) : jurisdiction === undefined || rules === null ? (
            <p className="mt-1 text-caption text-muted">Reading the source&apos;s jurisdiction…</p>
          ) : rule ? (
            <ClockRuleSummary rule={rule} />
          ) : jurisdiction === 'NONE' ? (
            <p className="mt-1 text-caption leading-relaxed text-muted">
              This source is marked as under no modeled statute, so no deadline is evaluated.
            </p>
          ) : (
            <p className="mt-1 text-caption leading-relaxed text-muted">
              No jurisdiction is set for this source, so no statutory deadline is evaluated. No statute is assumed.
            </p>
          )}
        </div>
        {rules && jurisdiction !== undefined && !loadError && (
          <label className="shrink-0 text-caption font-semibold text-muted">
            <span className="sr-only">Statute for this source</span>
            <select
              aria-label="Statute for this source"
              value={jurisdiction ?? ''}
              disabled={isSaving}
              onChange={(event) => onSelect(event.target.value as 'CA_CPRA' | 'US_FOIA' | 'NONE')}
              className="rounded-md border border-control bg-canvas px-2 py-1 text-caption text-fg-secondary"
            >
              {jurisdiction === null && <option value="">Not set</option>}
              {rules.map((candidate) => (
                <option key={candidate.id} value={candidate.jurisdiction}>
                  {candidate.title}
                </option>
              ))}
              <option value="NONE">No modeled statute</option>
            </select>
          </label>
        )}
      </div>
    </div>
  );
}
