import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { DossierGap, EvidenceLink, LeadDossier as LeadDossierData } from '../../types';
import { findingToBriefingEvidence } from './BriefingWorkspace';
import ArtifactDrafts from './ArtifactDrafts';

type DossierState =
  | { status: 'pending' }
  | { status: 'ready'; dossier: LeadDossierData }
  | { status: 'failed'; message: string };

/**
 * Assembles one finding into a dossier: the records it cites, the findings that touch the same
 * request or documents, what would disconfirm it, and which figures in the cited documents could
 * not be checked. Every fetch is gated on the finding it was requested for.
 */
export default function LeadDossier() {
  const findingId = useAppStore((state) => state.dossierFindingId);
  const close = useAppStore((state) => state.closeLeadDossier);
  const addBriefingEvidence = useAppStore((state) => state.addBriefingEvidence);
  const [state, setState] = useState<DossierState>({ status: 'pending' });
  const gate = useLatestRequestGate();

  useEffect(() => {
    if (!findingId) return;
    const request = gate.begin();
    setState({ status: 'pending' });
    invokeCommand('build_lead_dossier', { findingId })
      .then((dossier) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'ready', dossier });
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'failed', message: formatNativeError(reason) });
      });
    return gate.invalidate;
  }, [findingId, gate]);

  if (!findingId) return null;

  const addToBriefing = (dossier: LeadDossierData) => {
    const item = findingToBriefingEvidence(dossier.finding);
    if (item) addBriefingEvidence(item);
  };

  const exportMarkdown = (dossier: LeadDossierData) => {
    const url = URL.createObjectURL(new Blob([dossierMarkdown(dossier)], { type: 'text/markdown;charset=utf-8' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `lead-dossier-${dossier.finding.id}.md`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <section aria-label="Lead dossier" className="mt-4 rounded-xl border border-slate-800 bg-slate-950/40 p-4 text-label text-slate-300">
      <div className="flex items-start justify-between gap-3">
        <h3 className="text-label font-semibold text-slate-200">Lead dossier</h3>
        <button type="button" onClick={close} className="focus-ring rounded-md border border-slate-800 px-2 py-1 text-caption text-slate-400 hover:text-slate-200">Close</button>
      </div>
      {state.status === 'pending' && <p role="status" className="mt-2 text-slate-400">Assembling the dossier…</p>}
      {state.status === 'failed' && <p role="alert" className="mt-2 text-rose-300">The dossier could not be assembled: {state.message}</p>}
      {state.status === 'ready' && (
        <div className="mt-2 space-y-4">
          <div>
            <p className="font-semibold text-slate-200">{state.dossier.finding.headline}</p>
            <p className="mt-1 text-slate-400">{state.dossier.finding.explanation}</p>
          </div>
          <Section title={`Evidence chain (${state.dossier.evidence.length}${state.dossier.evidenceTruncated ? ', more not shown' : ''})`}>
            {state.dossier.evidence.length === 0 ? (
              <p className="text-slate-400">This finding cites no document span.</p>
            ) : (
              <ul className="space-y-1.5">
                {state.dossier.evidence.map((link, index) => <li key={`${link.portalKey}:${link.documentId}:${index}`}>{evidenceLine(link)}</li>)}
              </ul>
            )}
          </Section>
          <Section title="Draft from this finding">
            <ArtifactDrafts findingId={state.dossier.finding.id} />
          </Section>
          <Section title={`Related findings (${state.dossier.related.length}${state.dossier.relatedTruncated ? ', more not shown' : ''})`}>
            {state.dossier.related.length === 0 ? (
              <p className="text-slate-400">No other undismissed finding touches this request or these documents.</p>
            ) : (
              <ul className="space-y-1">
                {state.dossier.related.map((related) => (
                  <li key={related.id}>
                    <span className="text-slate-400">{related.basis === 'SAME_REQUEST' ? 'same request' : 'same document'}</span> · {related.headline}
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <Section title="What would disconfirm this">
            <p>{state.dossier.falsification.statement}</p>
            <p className="mt-1 text-slate-400">Records to seek:</p>
            <ul className="list-disc pl-4 text-slate-400">
              {state.dossier.falsification.recordsToSeek.map((record) => <li key={record}>{record}</li>)}
            </ul>
          </Section>
          <Section title={`Gaps (${state.dossier.gaps.length}${state.dossier.gapsTruncated ? ', more not shown' : ''})`}>
            {state.dossier.gaps.length === 0 ? (
              <p className="text-slate-400">Every figure found in the cited documents could be checked against the imported tables, or none was found.</p>
            ) : (
              <ul className="space-y-2">
                {state.dossier.gaps.map((gap, index) => (
                  <li key={index}>
                    <p>{gapLine(gap)}</p>
                    <GapRequest gap={gap} />
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={() => addToBriefing(state.dossier)} className="focus-ring rounded-md border border-indigo-400/25 bg-indigo-500/10 px-2 py-1 text-caption font-semibold text-indigo-200">Add to briefing</button>
            <button type="button" onClick={() => exportMarkdown(state.dossier)} className="focus-ring rounded-md border border-slate-800 px-2 py-1 text-caption font-medium text-slate-400 hover:text-slate-200">Export as Markdown</button>
          </div>
        </div>
      )}
    </section>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h4 className="text-caption font-semibold uppercase tracking-wider text-cyan-300/80">{title}</h4>
      <div className="mt-1">{children}</div>
    </div>
  );
}

function evidenceLine(link: EvidenceLink): string {
  const name = link.fileName ?? `document ${link.documentId} (not resolved)`;
  const span = link.characterStart === null ? '' : ` · characters ${link.characterStart}–${link.characterEnd ?? link.characterStart}`;
  const agency = link.agencyLabel ? ` · ${link.agencyLabel}` : '';
  const digest = link.rawFileSha256 ? ` · sha256 ${link.rawFileSha256.slice(0, 12)}…` : '';
  const quote = link.quote ? ` — “${link.quote}”` : '';
  return `${link.portalKey} / ${name}${agency}${span}${digest}${quote}`;
}

type CopyState = { kind: 'idle' } | { kind: 'copied' } | { kind: 'fallback'; text: string };

/**
 * Offers the records request that would close one gap. The text asks for the records that would
 * confirm or refute the figure; it never states that the claim is wrong. Composed locally from the
 * gap's own fields, so nothing in it goes beyond what the dossier recorded.
 */
function GapRequest({ gap }: { gap: DossierGap }) {
  const [copy, setCopy] = useState<CopyState>({ kind: 'idle' });

  const copyRequest = async () => {
    const text = gapRequestText(gap);
    try {
      await navigator.clipboard.writeText(text);
      setCopy({ kind: 'copied' });
    } catch {
      setCopy({ kind: 'fallback', text });
    }
  };

  return (
    <div className="mt-1">
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => void copyRequest()}
          className="focus-ring rounded-md border border-slate-800 px-2 py-0.5 text-caption font-medium text-slate-400 hover:text-slate-200"
        >
          Copy request text
        </button>
        {copy.kind === 'copied' && <span role="status" className="text-caption text-emerald-300">Request text copied.</span>}
      </div>
      {copy.kind === 'fallback' && (
        <>
          <p role="status" className="mt-1 text-caption text-amber-200">Copying was blocked by the system. Select the text below to copy it.</p>
          <pre className="mt-1 whitespace-pre-wrap rounded-md border border-slate-800 bg-slate-950/60 p-2 text-caption text-slate-300">{copy.text}</pre>
        </>
      )}
    </div>
  );
}

/** A neutral public-records request paragraph for the records that would close `gap`. */
export function gapRequestText(gap: DossierGap): string {
  const record = `document ${gap.documentId} held under ${gap.portalKey}`;
  if (gap.gap === 'notComputed') {
    return (
      `I request a complete, readable electronic copy of ${record}, together with its metadata ` +
      `(creation and revision dates, author or originating office, and the source of any figures it contains). ` +
      `The copy available to me could not be examined (${gap.reason}), so the figures in it could not be checked ` +
      `against the imported records. Please provide the record in a form that allows those figures to be confirmed or refuted.`
    );
  }
  return (
    `I request the dataset, tables, or working figures that support the following statement in ${record}: ` +
    `"${gap.claimText}". The statement was compared against the imported records and could not be reconciled ` +
    `(${gap.reason}: ${gap.detail}). Please provide the underlying records, including the source data, its date, ` +
    `and any revision it reflects, in an electronic format if available, so that the stated figure can be confirmed or refuted.`
  );
}

function gapLine(gap: DossierGap): string {
  if (gap.gap === 'notComputed') return `${gap.portalKey} / document ${gap.documentId}: figures not examined — ${gap.reason}`;
  return `${gap.portalKey} / document ${gap.documentId}: “${gap.claimText}” — ${gap.reason}: ${gap.detail}`;
}

export function dossierMarkdown(dossier: LeadDossierData): string {
  const lines = [
    `# Lead dossier: ${dossier.finding.headline}`,
    '',
    dossier.finding.explanation,
    '',
    '## Evidence chain',
    ...(dossier.evidence.length ? dossier.evidence.map((link) => `- ${evidenceLine(link)}`) : ['- This finding cites no document span.']),
    '',
    '## Related findings',
    ...(dossier.related.length ? dossier.related.map((related) => `- ${related.headline}`) : ['- None.']),
    '',
    '## What would disconfirm this',
    dossier.falsification.statement,
    ...dossier.falsification.recordsToSeek.map((record) => `- ${record}`),
    '',
    '## Gaps',
    ...(dossier.gaps.length ? dossier.gaps.map((gap) => `- ${gapLine(gap)}`) : ['- Every figure found could be checked, or none was found.']),
    '',
    `Assembled ${dossier.assembledAt}. Observations only; a pattern signal is not a legal claim.`,
  ];
  return lines.join('\n');
}
